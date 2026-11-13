// The editor side of ADR 0012/0013. Revisions are longer than continuations
// and never stream, carry through typing, or accept one word at a time.
export const REVISION_TIMEOUT_MS = 45000;
export const revisionModes = new Set(['grammar', 'refine', 'synonyms']);
export const validRevisionText = (text, limit = 1100) => typeof text === 'string' &&
  [...text].length >= 1 && [...text].length <= limit &&
  !/[\p{Cc}\p{Cf}\p{Cs}\p{Zl}\p{Zp}]/u.test(text) &&
  ![...text].some(character => /\s/u.test(character) && ![' ', '\u00a0', '\u202f'].includes(character));

export const sameSelection = (a, b) => a?.unit === 'unicode_scalar_values' && b?.unit === a.unit &&
  a.anchor === b.anchor && a.head === b.head;

export function validSelection(selection, selected) {
  return selection?.unit === 'unicode_scalar_values' &&
    Number.isSafeInteger(selection.anchor) && selection.anchor >= 0 &&
    Number.isSafeInteger(selection.head) && selection.head >= 0 &&
    Math.abs(selection.head - selection.anchor) === [...selected].length;
}

export function synonymSelection(selected) {
  const words = selected.trim().split(/\s+/u);
  return validRevisionText(selected.trim(), 600) && words.length >= 1 && words.length <= 3 &&
    words.every(word => /\p{L}/u.test(word)) && !/[0-9@:.?!;]/u.test(selected);
}

export function nextRevisionMode(mode, selected) {
  if (mode === 'grammar') return 'refine';
  if (mode === 'synonyms') return 'synonyms';
  return synonymSelection(selected) ? 'synonyms' : 'grammar';
}

// Preserve every character, including spacing, in the marked preview. A
// common prefix/suffix of words stays unmarked; the changed span is shown
// removed then added. This bounds work even for a 600-character selection.
export function revisionDiff(original, revised) {
  const split = text => text.match(/\S+\s*|\s+/gu) ?? [];
  const from = split(original), to = split(revised);
  let start = 0;
  while (start < from.length && start < to.length && from[start] === to[start]) start++;
  let end = 0;
  while (end < from.length - start && end < to.length - start &&
    from[from.length - 1 - end] === to[to.length - 1 - end]) end++;
  return [
    { kind: 'same', text: from.slice(0, start).join('') },
    { kind: 'removed', text: from.slice(start, from.length - end).join('') },
    { kind: 'added', text: to.slice(start, to.length - end).join('') },
    { kind: 'same', text: end ? from.slice(-end).join('') : '' },
  ].filter(part => part.text);
}
