import { StateEffect, StateField } from '@codemirror/state';
import { showTooltip, getTooltip, tooltips } from '@codemirror/view';
import { boundedContext } from '../shared/broker-client.mjs';
import { validRevisionText, revisionDiff } from '../shared/revision.mjs';

export const setRevisionTooltip = StateEffect.define();
export const revisionTooltip = StateField.define({
  create: () => null,
  update(value, transaction) {
    if (transaction.docChanged || transaction.selection) value = null;
    for (const effect of transaction.effects) if (effect.is(setRevisionTooltip)) value = effect.value;
    return value;
  },
  provide: field => showTooltip.from(field),
});

function revisionSpace(view) {
  const viewport = view.dom.ownerDocument.documentElement;
  const editor = view.scrollDOM.getBoundingClientRect();
  return { top: Math.max(0, editor.top), left: Math.max(0, editor.left),
    bottom: Math.min(viewport.clientHeight, editor.bottom), right: Math.min(viewport.clientWidth, editor.right) };
}

// Obsidian clips fixed tooltips to its editor pane. Let CodeMirror flip the
// whole revision below a top-line selection instead of hiding its first lines
// behind the toolbar. Ordinary host tooltips retain the default viewport.
export const revisionTooltipSpace = tooltips({ tooltipSpace: view => {
  if (view.state.field(revisionTooltip, false)) return revisionSpace(view);
  const viewport = view.dom.ownerDocument.documentElement;
  return { top: 0, left: 0, bottom: viewport.clientHeight, right: viewport.clientWidth };
} });

// CodeMirror owns UTF-16 positions; the broker binds scalar positions on the
// selected line. Keep both, and never round an endpoint inside a surrogate pair.
export function captureRevision(state) {
  const { doc, selection } = state;
  if (selection.ranges.length !== 1 || selection.main.empty) return null;
  const { from, to, anchor, head } = selection.main;
  if (to - from > 1200) return null;
  const selected = doc.sliceString(from, to);
  if (!validRevisionText(selected, 600) || !selected.trim()) return null;
  const line = doc.lineAt(from);
  if (to > line.to) return null;
  const offset = [...doc.sliceString(line.from, from)].length;
  const end = offset + [...selected].length;
  const rawBefore = doc.sliceString(Math.max(0, from - 1024), from).replace(/^[\uDC00-\uDFFF]/u, '');
  const rawAfter = doc.sliceString(to, Math.min(line.to, to + 256)).replace(/[\uD800-\uDBFF]$/u, '');
  return { kind: 'revision', doc, selection, from, to, selected, ...boundedContext(rawBefore, rawAfter),
    range: { anchor: anchor > head ? end : offset, head: anchor > head ? offset : end,
      unit: 'unicode_scalar_values' } };
}

export function markedRevision(snapshot, text, mode) {
  return { pos: snapshot.selection.main.head, end: snapshot.to, above: true, strictSide: false,
    create(view) {
      const document = view.dom.ownerDocument;
      const dom = document.createElement('div');
      dom.className = 'badi-revision';
      dom.setAttribute('role', 'status');
      dom.setAttribute('aria-label', `Badi ${mode} revision. Tab applies the whole change; Escape dismisses.`);
      const title = document.createElement('div');
      title.className = 'badi-revision-title';
      title.textContent = { grammar: 'Grammar', refine: 'Refine wording', synonyms: 'Another word' }[mode];
      dom.append(title);
      const diff = document.createElement('div');
      diff.className = 'badi-revision-diff';
      for (const part of revisionDiff(snapshot.selected, text)) {
        const span = document.createElement('span');
        span.className = `badi-revision-${part.kind}`;
        span.textContent = part.kind === 'removed' ? `[-${part.text}]`
          : part.kind === 'added' ? `[+${part.text}]` : part.text;
        diff.append(span);
      }
      const hint = document.createElement('div');
      hint.className = 'badi-revision-hint';
      hint.textContent = 'Tab applies · Escape dismisses · Ctrl+Shift+Space tries another';
      dom.append(diff, hint);
      // The host normally shrinks tooltips that do not fit. A partially
      // displayed rewrite must never become an acceptable edit.
      return { dom, resize: false };
    } };
}

export function revisionFullyVisible(view, tooltip) {
  const dom = getTooltip(view, tooltip)?.dom;
  if (!dom?.isConnected) return false;
  const rect = dom.getBoundingClientRect();
  const anchor = view.coordsAtPos(tooltip.pos);
  const space = revisionSpace(view);
  return !!anchor && anchor.bottom > space.top && anchor.top < space.bottom
    && anchor.right >= space.left && anchor.left <= space.right
    && rect.width > 0 && rect.height > 0 && rect.left >= space.left && rect.top >= space.top
    && rect.right <= space.right && rect.bottom <= space.bottom
    && dom.scrollWidth <= dom.clientWidth && dom.scrollHeight <= dom.clientHeight;
}
