import { Plugin, Notice } from 'obsidian';
import { Decoration, ViewPlugin, WidgetType } from '@codemirror/view';
import { Prec, Transaction } from '@codemirror/state';
import { isolateHistory } from '@codemirror/commands';
import { completionStatus } from '@codemirror/autocomplete';
import { boundedContext } from '../shared/broker-client.mjs';
import { RecoveringBrokerClient } from '../shared/recovering-client.mjs';
import { recordActivity } from '../shared/activity.mjs';
import { writingLanguage } from '../shared/writing-language.mjs';
import { validCorrection } from '../shared/text-safety.mjs';
import { nextRevisionMode, revisionModes, sameSelection, validRevisionText } from '../shared/revision.mjs';
import { captureRevision, markedRevision, revisionFullyVisible, revisionTooltip, revisionTooltipSpace, setRevisionTooltip } from './revision.mjs';

// CodeMirror positions are UTF-16 code units. A fixed-size pre-slice can
// split an astral character; drop the orphan half before scalar bounding.
const dropSplitSurrogate = (value, edge) => edge === 'start'
  ? value.replace(/^[\uDC00-\uDFFF]/u, '')
  : value.replace(/[\uD800-\uDBFF]$/u, '');

class SuggestionWidget extends WidgetType {
  // `word` is what Tab takes, drawn a step brighter when more follows it.
  // A `list` widget shows numbered alternative words instead (ADR 0014).
  constructor(text, word = '', list = false) { super(); this.text = text; this.word = word; this.list = list; }
  eq(other) { return other.text === this.text && other.word === this.word && other.list === this.list; }
  toDOM() {
    const element = document.createElement('span');
    element.className = 'badi-inline';
    if (this.word && this.word.length < this.text.length && this.text.startsWith(this.word)) {
      const word = document.createElement('span');
      word.className = 'badi-inline-word';
      word.textContent = this.word;
      element.append(word, this.text.slice(this.word.length));
    } else {
      element.textContent = this.text;
    }
    element.setAttribute('role', 'status');
    if (this.list) {
      element.setAttribute('aria-label', `Badi alternatives: ${this.text}. Press a number to pick; Escape dismisses.`);
      element.title = 'Badi · 1–9 pick a word · Escape dismiss';
    } else {
      element.setAttribute('aria-label', `Badi suggestion: ${this.text}. Tab accepts a word; Control Right accepts all; Escape dismisses.`);
      element.title = 'Badi · Tab next word · Ctrl/⌘ Right all · Escape dismiss';
    }
    return element;
  }
  ignoreEvent() { return true; }
}

class WritingView {
  decorations = Decoration.none;
  timer = null;
  pending = false;
  text = null;
  listWords = null;
  replaceBefore = undefined;
  snapshot = null;
  destroyed = false;
  applying = false;
  generation = 0;
  renderQueued = false;
  revision = null;
  revisionCycle = null;
  revisionReason = null;
  changingRevision = false;
  observeEscape = event => {
    this.dismiss(event);
    if ((this.snapshot?.kind === 'revision' || this.revisionCycle) && event.isTrusted && !event.isComposing &&
        !['Control', 'Shift', 'Meta', 'Alt'].includes(event.key) &&
        !(event.ctrlKey && event.shiftKey && !event.metaKey && !event.altKey && event.key === ' ') &&
        !(event.key === 'Tab' && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey)) this.invalidate();
  };
  observeBlur = () => this.invalidate();

  constructor(view, owner) {
    this.view = view;
    this.owner = owner;
    owner.views.set(view, this);
    // Obsidian prevents Escape before its bundled CodeMirror dispatches even
    // event observers. Observe this editor's DOM without consuming the event.
    view.contentDOM.addEventListener('keydown', this.observeEscape, { passive: true });
    view.dom.ownerDocument.defaultView?.addEventListener('blur', this.observeBlur);
    // An editor opened while the shared client is ready gets no 'ready' event.
    owner.connect();
    queueMicrotask(() => this.scheduleAutomatic());
  }

  // Every editor shares the plugin's one broker connection.
  get client() { return this.owner.client; }

  clientState(state) {
    if (this.owner.session === this && this.pending && this.snapshot?.kind === 'revision') this.revisionReason = state;
    if (state === 'offline' || state === 'offline_retry') this.invalidate();
    if (state === 'ready') queueMicrotask(() => this.scheduleAutomatic());
  }

  scheduleAutomatic() {
    if (this.eligible() && !this.pending && !this.text && !this.listWords) {
      clearTimeout(this.timer);
      this.timer = setTimeout(() => void this.request(false), 250);
    }
  }

  clearPreview() {
    if (this.text) recordActivity('obsidian', 'ghost', 'hidden');
    this.text = null;
    this.listWords = null;
    this.replaceBefore = undefined;
    this.revision = null;
    this.decorations = Decoration.none;
    this.render();
  }

  render() {
    if (this.destroyed || this.renderQueued) return;
    this.renderQueued = true;
    queueMicrotask(() => {
      this.renderQueued = false;
      if (!this.destroyed) this.view.dispatch({ effects: setRevisionTooltip.of(this.revision?.tooltip ?? null) });
    });
  }

  invalidate() {
    this.generation++;
    clearTimeout(this.timer);
    this.pending = false;
    this.snapshot = null;
    this.revisionCycle = null;
    this.clearPreview();
    // The client holds one session at a time; only the editor that opened it
    // may cancel it, so an edit elsewhere never withdraws this one's preview.
    if (this.owner.session === this) this.client.cancel();
  }

  blockedReason(revising = false) {
    const { selection, doc, readOnly } = this.view.state;
    if (this.destroyed) return 'editor_closed';
    if (!this.client.connected) return 'model_offline';
    if (!this.client.allowed) return 'app_disabled_or_model_paused';
    if (!this.view.hasFocus || !this.view.dom.ownerDocument.hasFocus()) return 'editor_not_focused';
    if (this.view.composing) return 'input_method_composing';
    if (completionStatus(this.view.state) !== null) return 'editor_completion_active';
    if (readOnly) return 'read_only';
    if (selection.ranges.length !== 1 || (revising ? selection.main.empty : !selection.main.empty))
      return revising ? 'select_text' : 'selection_active';
    return doc.length ? null : 'empty_note';
  }

  eligible() { return this.blockedReason() === null; }

  matches(snapshot) {
    return this.blockedReason(snapshot?.kind === 'revision') === null && this.view.state.doc === snapshot.doc &&
      this.view.state.selection.eq(snapshot.selection);
  }

  update(update) {
    if (this.applying) return;
    if (update.docChanged) recordActivity('obsidian', 'context', 'context_received');
    if (update.docChanged || (update.selectionSet && !update.startState.selection.eq(update.state.selection)) ||
        (update.focusChanged && !this.view.hasFocus)) this.invalidate();
    if (update.docChanged && this.eligible()) this.timer = setTimeout(() => void this.request(false), 250);
    if ((update.docChanged || update.focusChanged) && this.view.hasFocus && !this.client.connected) {
      void this.client.connect().catch(() => undefined);
    }
  }

  async request(explicit = true) {
    if (explicit && !this.view.state.selection.main.empty) return this.revise();
    const reason = this.blockedReason();
    recordActivity('obsidian', 'decision', reason ?? 'eligible');
    if (reason || this.pending || (!explicit && !this.client.automaticAllowed)) return;
    clearTimeout(this.timer);
    const snapshot = { doc: this.view.state.doc, selection: this.view.state.selection };
    const end = snapshot.selection.main.head;
    // Text after the caret lets the broker suggest only what fits before it.
    // UTF-16 slices of twice the scalar limits always hold enough scalars.
    const beforeStart = Math.max(0, end - 1024);
    const afterEnd = end + 256;
    const rawBefore = dropSplitSurrogate(
      snapshot.doc.sliceString(beforeStart, end), beforeStart > 0 ? 'start' : '');
    const rawAfter = dropSplitSurrogate(
      snapshot.doc.sliceString(end, afterEnd), afterEnd < snapshot.doc.length ? 'end' : '');
    const { before, after } = boundedContext(rawBefore, rawAfter);
    if (!before.trim()) return;
    this.pending = true;
    this.owner.session = this;
    const generation = this.generation;
    try {
      const language = writingLanguage(before, this.owner.language === 'auto' ? navigator.language : this.owner.language);
      if (!language) throw new Error('Writing language unavailable');
      const text = await this.client.suggest(before, { explicit, language, after });
      if (generation !== this.generation || !this.matches(snapshot) || !text) return;
      const replaceBefore = this.client.replacementBefore;
      if (replaceBefore !== undefined && !validCorrection(before, replaceBefore, text)) {
        this.client.cancel();
        return;
      }
      this.snapshot = snapshot;
      this.text = text;
      this.replaceBefore = replaceBefore;
      this.showPreview(replaceBefore === undefined ? text : ` ${replaceBefore.trim()} → ${text.trim()}`,
        replaceBefore === undefined ? this.client.acceptWord : '');
    } catch {
      if (generation === this.generation && this.view.hasFocus) this.owner.status.setText('Badi · no suggestion · Ctrl+Shift+Space retries');
    } finally {
      if (generation === this.generation) this.pending = false;
    }
  }

  async revise(mode) {
    const reason = this.blockedReason(true);
    if (reason || this.pending) return;
    const snapshot = captureRevision(this.view.state);
    if (!snapshot) { this.owner.status.setText('Badi · select 1–600 characters on one line'); return; }
    mode ??= this.revisionCycle && this.matches(this.revisionCycle.snapshot) ? this.revisionCycle.next : 'grammar';
    if (!revisionModes.has(mode)) return;
    clearTimeout(this.timer);
    this.generation++;
    const generation = this.generation;
    this.clearPreview();
    this.snapshot = snapshot;
    this.pending = true;
    this.revisionReason = null;
    this.owner.session = this;
    this.owner.status.setText('Badi · revising selection');
    try {
      // revise() retires its own previous preview synchronously while keeping
      // the exact session for synonyms. Other clears still revoke this view.
      this.changingRevision = true;
      const promise = this.client.revise({ before: snapshot.before, selected: snapshot.selected, after: snapshot.after,
        selection: snapshot.range, mode,
        language: this.owner.language === 'auto' ? navigator.language : this.owner.language });
      this.changingRevision = false;
      const result = await promise;
      if (generation !== this.generation || !this.matches(snapshot)) return;
      if (!result && this.revisionReason !== 'no_change') { this.invalidate(); return; }
      this.revisionCycle = { snapshot, next: nextRevisionMode(mode, snapshot.selected) };
      if (!result) { this.owner.status.setText('Badi · no change · Ctrl+Shift+Space tries another'); return; }
      if (result.original !== snapshot.selected || result.mode !== mode || !validRevisionText(result.text)) {
        this.invalidate(); return;
      }
      this.text = result.text;
      this.revision = { mode, tooltip: markedRevision(snapshot, result.text, mode) };
      this.render();
    } catch {
      if (generation === this.generation) { this.invalidate(); this.owner.status.setText('Badi · revision request failed · retry'); }
    } finally {
      this.changingRevision = false;
      if (generation === this.generation) this.pending = false;
    }
  }

  async acceptRevision() {
    const snapshot = this.snapshot, revision = this.revision, shown = this.text;
    if (!revision || !shown || this.pending || !this.matches(snapshot)) return;
    if (!revisionFullyVisible(this.view, revision.tooltip)) {
      this.invalidate(); this.owner.status.setText('Badi · revision does not fit on screen'); return;
    }
    this.pending = true;
    const generation = this.generation;
    try {
      const grant = await this.client.authorizeRevision();
      if (generation !== this.generation || !this.matches(snapshot) || this.revision !== revision ||
          grant.text !== shown || grant.replaceBefore !== undefined || !sameSelection(grant.replaceSelection, snapshot.range) ||
          !validRevisionText(grant.text) || !revisionFullyVisible(this.view, revision.tooltip)) {
        this.client.report(grant, 'stale'); return;
      }
      this.applying = true;
      try {
        this.view.dispatch({ changes: { from: snapshot.from, to: snapshot.to, insert: grant.text },
          selection: { anchor: snapshot.from + grant.text.length },
          annotations: [Transaction.userEvent.of('input.complete'), isolateHistory.of('full')] });
        const applied = this.view.state.doc.toString() === snapshot.doc.sliceString(0, snapshot.from) +
          grant.text + snapshot.doc.sliceString(snapshot.to) &&
          this.view.state.selection.main.empty && this.view.state.selection.main.head === snapshot.from + grant.text.length;
        this.client.report(grant, applied ? 'applied' : 'failed');
      } finally { this.applying = false; }
    } catch {
      if (generation === this.generation) this.owner.status.setText('Badi · revision changed · select and try again');
    } finally {
      if (generation === this.generation) this.invalidate();
    }
  }

  showPreview(preview, word = '', list = false) {
    const end = this.snapshot.selection.main.head;
    this.decorations = Decoration.set([Decoration.widget({ widget: new SuggestionWidget(preview, word, list), side: 1 }).range(end)]);
    this.render();
    recordActivity('obsidian', 'ghost', 'shown');
  }

  // Ctrl+Shift+Space on a shown continuation: one that starts with another
  // word replaces it. The shown one stays until the answer arrives; no
  // answer withdraws it.
  async alternative() {
    const snapshot = this.snapshot;
    if (!this.text || this.replaceBefore !== undefined || !snapshot || !this.matches(snapshot) || this.pending) return;
    this.pending = true;
    const generation = this.generation;
    try {
      const result = await this.client.alternative();
      if (generation !== this.generation || !this.matches(snapshot)) return;
      if (!result) { this.invalidate(); return; }
      if (typeof result === 'object') {
        // A numbered list of other first words; a digit picks one. It
        // stands where the shown suggestion stood, space included.
        const lead = this.text?.startsWith(' ') ? ' ' : '';
        this.text = null;
        this.listWords = result.words;
        this.showPreview(lead + result.words.map((word, index) => `${index + 1}.${word}`).join('  '), '', true);
        return;
      }
      this.text = result;
      this.showPreview(result, this.client.acceptWord);
    } catch {
      if (generation === this.generation) {
        this.invalidate();
        if (this.view.hasFocus) this.owner.status.setText('Badi · no other suggestion');
      }
    } finally {
      if (generation === this.generation) this.pending = false;
    }
  }

  // A digit on a shown numbered list: the picked word's continuation
  // replaces the list, accepted like any suggestion.
  async pick(word) {
    const snapshot = this.snapshot;
    if (!this.listWords || !snapshot || !this.matches(snapshot) || this.pending) return;
    this.pending = true;
    const generation = this.generation;
    try {
      const text = await this.client.pickAlternative(word);
      if (generation !== this.generation || !this.matches(snapshot)) return;
      if (!text) { this.invalidate(); return; }
      this.listWords = null;
      this.text = text;
      this.showPreview(text, this.client.acceptWord);
    } catch {
      if (generation === this.generation) {
        this.invalidate();
        if (this.view.hasFocus) this.owner.status.setText('Badi · no other suggestion');
      }
    } finally {
      if (generation === this.generation) this.pending = false;
    }
  }

  // The broker streamed a longer version of the shown continuation.
  extend(text) {
    if (!this.text || this.replaceBefore !== undefined || this.pending || !this.matches(this.snapshot) ||
        !text.startsWith(this.text)) return;
    this.text = text;
    this.showPreview(text, this.client.acceptWord);
  }

  async accept(acceptance = 'word') {
    if (this.revision) return this.acceptRevision();
    const snapshot = this.snapshot;
    if (!this.text || !snapshot || !this.matches(snapshot) || this.pending) return;
    this.pending = true;
    const generation = this.generation;
    const replaceBefore = this.replaceBefore;
    const shown = this.text;
    let inserted = false;
    try {
      const grant = await this.client.authorize(acceptance);
      // The client may hold a streamed continuation this editor declined to
      // show; a grant inserts only what is on screen (all of it, or its start).
      const showsGrant = (acceptance === 'all' || replaceBefore !== undefined)
        ? grant.text === shown : typeof grant.text === 'string' && grant.text !== '' && shown.startsWith(grant.text);
      const end = snapshot.selection.main.head;
      if (generation !== this.generation || !this.matches(snapshot) || grant.replaceBefore !== replaceBefore || !showsGrant ||
          (replaceBefore !== undefined && !validCorrection(snapshot.doc.sliceString(0, end), replaceBefore, grant.text))) {
        this.client.report(grant, 'stale'); return;
      }
      const from = end - (replaceBefore?.length ?? 0);
      this.applying = true;
      try {
        this.view.dispatch({ changes: { from, to: end, insert: grant.text }, selection: { anchor: from + grant.text.length },
          annotations: [Transaction.userEvent.of('input.complete'), isolateHistory.of('full')] });
        const applied = this.view.state.doc.toString() ===
            snapshot.doc.sliceString(0, from) + grant.text + snapshot.doc.sliceString(end) &&
          this.view.state.selection.main.head === from + grant.text.length;
        this.client.report(grant, applied ? 'applied' : 'failed');
        inserted = applied;
      } finally { this.applying = false; }
    } catch {
      if (generation === this.generation) this.owner.status.setText('Badi · suggestion changed · Ctrl+Shift+Space retries');
    } finally {
      if (generation === this.generation) {
        this.invalidate();
        if (inserted && this.eligible()) this.timer = setTimeout(() => void this.request(false), 250);
      }
    }
  }

  dismiss(event) {
    if (this.destroyed || event.key !== 'Escape' || !event.isTrusted || event.isComposing || this.view.composing ||
        event.altKey || event.shiftKey || event.ctrlKey || event.metaKey ||
        !this.view.hasFocus || !this.view.dom.ownerDocument.hasFocus()) return;
    if (this.text || this.listWords) {
      this.client.dismiss();
      this.generation++;
      clearTimeout(this.timer);
      this.pending = false;
      this.snapshot = null;
      this.revisionCycle = null;
      this.clearPreview();
      return;
    }
    if (this.pending || this.revisionCycle) this.invalidate();
  }

  key(event) {
    if (!event.isTrusted || event.isComposing || event.altKey) return false;
    if (event.ctrlKey && event.shiftKey && !event.metaKey && event.key === ' ') {
      if (event.repeat || this.pending) return true;
      if (!this.view.state.selection.main.empty) void this.revise();
      else if (this.text) void this.alternative(); else void this.request();
      return true;
    }
    if (event.shiftKey) return false;
    if ((event.ctrlKey || event.metaKey) && event.key === 'ArrowRight' && this.text && this.eligible()) {
      if (!event.repeat) void this.accept('all');
      return true;
    }
    if (event.ctrlKey || event.metaKey) return false;
    if (this.listWords && /^[1-9]$/u.test(event.key)) {
      const word = this.listWords[Number(event.key) - 1];
      if (!word) return false;
      if (!event.repeat && !this.pending) void this.pick(word);
      return true;
    }
    if (event.key !== 'Tab') return false;
    if (!this.text) return false;
    const reason = this.blockedReason(!!this.revision);
    recordActivity('obsidian', 'tab', reason ?? 'eligible');
    if (reason) { this.owner.status.setText(`Badi · ${reason.replaceAll('_', ' ')}`); return false; }
    if (event.repeat || this.pending) return true;
    void this.accept();
    return true;
  }

  destroy() {
    this.destroyed = true;
    this.view.contentDOM.removeEventListener('keydown', this.observeEscape);
    this.view.dom.ownerDocument.defaultView?.removeEventListener('blur', this.observeBlur);
    this.invalidate();
    if (this.owner.session === this) this.owner.session = null;
    this.owner.views.delete(this.view);
  }
}

export default class BadiPlugin extends Plugin {
  views = new Map();
  language = 'auto';
  client = null;
  // The editor whose request opened the client's current session.
  session = null;

  createClient() {
    const client = new RecoveringBrokerClient('obsidian', { textReplacement: true, selectionRevision: true });
    client.on('state', state => {
      if (this.client !== client) return;
      if ([...this.views.keys()].some(view => view.hasFocus)) this.status.setText(state === 'revision_unavailable'
        ? 'Badi · revision unavailable · run badi revise' : `Badi · ${state.replaceAll('_', ' ')}`);
      for (const controller of this.views.values()) controller.clientState(state);
    });
    // A cancelled session withdraws whichever preview it produced.
    client.on('clear', () => {
      if (this.client !== client) return;
      for (const controller of this.views.values()) {
        if (controller.snapshot?.kind === 'revision' && !controller.changingRevision) {
          // Local revocation, not another cancel() call from this event.
          controller.generation++; controller.pending = false; controller.snapshot = null; controller.revisionCycle = null;
        }
        controller.clearPreview();
      }
    });
    client.on('update', text => { if (this.client === client) this.session?.extend(text); });
    return client;
  }

  connect() {
    const client = this.client;
    void client.connect().catch(() => {
      if (this.client === client && this.views.size) this.status.setText('Badi · offline · run badi doctor');
    });
  }

  reconnect() {
    for (const controller of this.views.values()) controller.invalidate();
    this.client.close();
    this.session = null;
    this.client = this.createClient();
    this.connect();
  }

  async onload() {
    let saved;
    try { saved = await this.loadData(); }
    catch { new Notice('Badi could not read its language preference; using the application language.'); }
    if (['auto', 'en', 'de', 'fa'].includes(saved?.language)) this.language = saved.language;
    this.status = this.addStatusBarItem();
    this.status.setText('Badi · starting');
    this.client = this.createClient();
    this.registerEditorExtension(revisionTooltip);
    this.registerEditorExtension(Prec.highest(revisionTooltipSpace));
    const owner = this;
    this.registerEditorExtension(Prec.highest(ViewPlugin.fromClass(class extends WritingView {
      constructor(view) { super(view, owner); }
    }, { decorations: value => value.decorations, eventHandlers: {
      keydown(event) { return this.key(event); },
      compositionstart() { this.invalidate(); return false; },
    } })));
    this.addCommand({ id: 'request-or-accept', name: 'Request or accept words', editorCallback: (_editor, markdown) => {
      // Obsidian exposes its CodeMirror view at editor.cm outside its typed Editor API.
      const controller = this.views.get(markdown.editor.cm);
      if (!controller || controller.blockedReason(!controller.view.state.selection.main.empty)) {
        new Notice('Badi: enable Obsidian with badi app obsidian on, then focus text in a note.'); return;
      }
      if (controller.text) void controller.accept(); else void controller.request();
    } });
    for (const [mode, label] of [['grammar', 'Check selected grammar'], ['refine', 'Refine selected wording'], ['synonyms', 'Find a synonym']]) {
      this.addCommand({ id: `revise-${mode}`, name: label, editorCallback: (_editor, markdown) => {
        void this.views.get(markdown.editor.cm)?.revise(mode);
      } });
    }
    this.addCommand({ id: 'retry-connection', name: 'Reconnect local model', callback: () => this.reconnect() });
    for (const [language, label] of [['auto', 'the application language'], ['en', 'English'], ['de', 'German'], ['fa', 'Persian']]) {
      this.addCommand({ id: `language-${language}`, name: `Use ${label} for suggestions`, callback: async () => {
        try {
          await this.saveData({ language });
          this.language = language;
          for (const controller of this.views.values()) controller.invalidate();
          new Notice(`Badi: suggestions use ${label}.`);
        } catch { new Notice('Badi could not save the language preference.'); }
      } });
    }
  }
  onunload() {
    for (const controller of this.views.values()) controller.destroy();
    this.client?.close();
  }
}
