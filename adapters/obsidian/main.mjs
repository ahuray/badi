import { Plugin, Notice } from 'obsidian';
import { Decoration, ViewPlugin, WidgetType } from '@codemirror/view';
import { Prec, Transaction } from '@codemirror/state';
import { isolateHistory } from '@codemirror/commands';
import { completionStatus } from '@codemirror/autocomplete';
import { RecoveringBrokerClient } from '../shared/recovering-client.mjs';
import { recordActivity } from '../shared/activity.mjs';
import { writingLanguage } from '../shared/writing-language.mjs';
import { validCorrection } from '../shared/text-safety.mjs';

class SuggestionWidget extends WidgetType {
  constructor(text) { super(); this.text = text; }
  eq(other) { return other.text === this.text; }
  toDOM() {
    const element = document.createElement('span');
    element.className = 'badi-inline';
    element.textContent = this.text;
    element.setAttribute('role', 'status');
    element.setAttribute('aria-label', `Badi suggestion: ${this.text}. Tab accepts a word; Control Right accepts all; Escape dismisses.`);
    element.title = 'Badi · Tab next word · Ctrl/⌘ Right all · Escape dismiss';
    return element;
  }
  ignoreEvent() { return true; }
}

class WritingView {
  decorations = Decoration.none;
  timer = null;
  pending = false;
  text = null;
  replaceBefore = undefined;
  snapshot = null;
  destroyed = false;
  applying = false;
  generation = 0;
  renderQueued = false;
  observeEscape = event => this.dismiss(event);

  constructor(view, owner) {
    this.view = view;
    this.owner = owner;
    owner.views.set(view, this);
    // Obsidian prevents Escape before its bundled CodeMirror dispatches even
    // event observers. Observe this editor's DOM without consuming the event.
    view.contentDOM.addEventListener('keydown', this.observeEscape, { passive: true });
    this.reconnect();
  }

  reconnect() {
    if (this.client) { this.invalidate(); this.client.close(); }
    this.client = new RecoveringBrokerClient('obsidian', { textReplacement: true });
    this.client.on('state', state => {
      if (this.view.hasFocus) this.owner.status.setText(`Badi · ${state.replaceAll('_', ' ')}`);
      if (state === 'offline' || state === 'offline_retry') this.invalidate();
      if (state === 'ready') queueMicrotask(() => {
        if (this.eligible() && !this.pending && !this.text) {
          clearTimeout(this.timer);
          this.timer = setTimeout(() => void this.request(false), 250);
        }
      });
    });
    this.client.on('clear', () => this.clearPreview());
    const client = this.client;
    void client.connect().catch(() => {
      if (!this.destroyed && this.client === client) this.owner.status.setText('Badi · offline · run badi doctor');
    });
  }

  clearPreview() {
    this.text = null;
    this.replaceBefore = undefined;
    this.decorations = Decoration.none;
    this.render();
  }

  render() {
    if (this.destroyed || this.renderQueued) return;
    this.renderQueued = true;
    queueMicrotask(() => {
      this.renderQueued = false;
      if (!this.destroyed) this.view.dispatch({});
    });
  }

  invalidate() {
    this.generation++;
    clearTimeout(this.timer);
    this.pending = false;
    this.snapshot = null;
    this.clearPreview();
    this.client.cancel();
  }

  blockedReason() {
    const { selection, doc, readOnly } = this.view.state;
    if (this.destroyed) return 'editor_closed';
    if (!this.client.connected) return 'model_offline';
    if (!this.client.allowed) return 'app_disabled_or_model_paused';
    if (!this.view.hasFocus || !this.view.dom.ownerDocument.hasFocus()) return 'editor_not_focused';
    if (this.view.composing) return 'input_method_composing';
    if (completionStatus(this.view.state) !== null) return 'editor_completion_active';
    if (readOnly) return 'read_only';
    if (selection.ranges.length !== 1 || !selection.main.empty) return 'selection_active';
    if (selection.main.head !== doc.length) return 'caret_not_at_note_end';
    return doc.length ? null : 'empty_note';
  }

  eligible() { return this.blockedReason() === null; }

  matches(snapshot) {
    return this.eligible() && this.view.state.doc === snapshot.doc &&
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
    const reason = this.blockedReason();
    recordActivity('obsidian', 'decision', reason ?? 'eligible');
    if (reason || this.pending || (!explicit && !this.client.automaticAllowed)) return;
    clearTimeout(this.timer);
    const snapshot = { doc: this.view.state.doc, selection: this.view.state.selection };
    const end = snapshot.selection.main.head;
    const before = [...snapshot.doc.sliceString(Math.max(0, end - 1024), end)].slice(-512).join('');
    if (!before.trim()) return;
    this.pending = true;
    const generation = this.generation;
    try {
      const language = writingLanguage(before, this.owner.language === 'auto' ? navigator.language : this.owner.language);
      if (!language) throw new Error('Writing language unavailable');
      const text = await this.client.suggest(before, { explicit, language });
      if (generation !== this.generation || !this.matches(snapshot) || !text) return;
      const replaceBefore = this.client.replacementBefore;
      if (replaceBefore !== undefined && !validCorrection(before, replaceBefore, text)) return;
      this.snapshot = snapshot;
      this.text = text;
      this.replaceBefore = replaceBefore;
      const preview = replaceBefore === undefined ? text : ` ${replaceBefore.trim()} → ${text.trim()}`;
      this.decorations = Decoration.set([Decoration.widget({ widget: new SuggestionWidget(preview), side: 1 }).range(end)]);
      this.render();
    } catch {
      if (generation === this.generation && this.view.hasFocus) this.owner.status.setText('Badi · no suggestion · Tab retries');
    } finally {
      if (generation === this.generation) this.pending = false;
    }
  }

  async accept(acceptance = 'word') {
    const snapshot = this.snapshot;
    if (!this.text || !snapshot || !this.matches(snapshot) || this.pending) return;
    this.pending = true;
    const generation = this.generation;
    const replaceBefore = this.replaceBefore;
    let inserted = false;
    try {
      const grant = await this.client.authorize(acceptance);
      if (generation !== this.generation || !this.matches(snapshot) || grant.replaceBefore !== replaceBefore ||
          (replaceBefore !== undefined && !validCorrection(snapshot.doc.sliceString(), replaceBefore, grant.text))) {
        this.client.report(grant, 'stale'); return;
      }
      const end = snapshot.selection.main.head;
      const from = end - (replaceBefore?.length ?? 0);
      this.applying = true;
      try {
        this.view.dispatch({ changes: { from, to: end, insert: grant.text }, selection: { anchor: from + grant.text.length },
          annotations: [Transaction.userEvent.of('input.complete'), isolateHistory.of('full')] });
        const applied = this.view.state.doc.sliceString() === snapshot.doc.sliceString(0, from) + grant.text &&
          this.view.state.selection.main.head === from + grant.text.length;
        this.client.report(grant, applied ? 'applied' : 'failed');
        inserted = applied;
      } finally { this.applying = false; }
    } catch {
      if (generation === this.generation) this.owner.status.setText('Badi · suggestion changed · Tab retries');
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
    if (this.text || this.pending) this.invalidate();
  }

  key(event) {
    if (!event.isTrusted || event.isComposing || event.altKey || event.shiftKey) return false;
    if ((event.ctrlKey || event.metaKey) && event.key === 'ArrowRight' && this.text && this.eligible()) {
      if (!event.repeat) void this.accept('all');
      return true;
    }
    if (event.ctrlKey || event.metaKey) return false;
    if (event.key !== 'Tab') return false;
    if (!this.client.connected) void this.client.connect().catch(() => undefined);
    const reason = this.blockedReason();
    recordActivity('obsidian', 'tab', reason ?? 'eligible');
    if (reason) { this.owner.status.setText(`Badi · ${reason.replaceAll('_', ' ')}`); return false; }
    if (event.repeat || this.pending) return true;
    if (this.text) void this.accept(); else void this.request();
    return true;
  }

  destroy() {
    this.destroyed = true;
    this.view.contentDOM.removeEventListener('keydown', this.observeEscape);
    this.invalidate();
    this.client.close();
    this.owner.views.delete(this.view);
  }
}

export default class BadiPlugin extends Plugin {
  views = new Map();
  language = 'auto';
  async onload() {
    let saved;
    try { saved = await this.loadData(); }
    catch { new Notice('Badi could not read its language preference; using the application language.'); }
    if (['auto', 'en', 'de', 'fa'].includes(saved?.language)) this.language = saved.language;
    this.status = this.addStatusBarItem();
    this.status.setText('Badi · starting');
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
      if (!controller?.eligible()) { new Notice('Badi: enable Obsidian with badi app obsidian on, then place the caret at the end of the note.'); return; }
      if (controller.text) void controller.accept(); else void controller.request();
    } });
    this.addCommand({ id: 'retry-connection', name: 'Reconnect local model', callback: () => {
      for (const controller of this.views.values()) controller.reconnect();
    } });
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
  onunload() { for (const controller of this.views.values()) controller.destroy(); }
}
