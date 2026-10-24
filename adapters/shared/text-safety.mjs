const arabicLetter = value => value !== undefined && /[\u0620-\u063f\u0641-\u064a\u066e-\u066f\u0671-\u06d3\u06d5\u06e5-\u06e6\u06ee-\u06ef\u06fa-\u06fc\u06ff]/u.test(value);

// V2 safe suggestion spacing: isolated U+0020, at most one leading and one
// trailing. The trailing space separates a mid-sentence insertion from the
// next word. Obsidian, Bash and Neovim display this text unchanged.
export const hasIsolatedSpaces = value => /^ ?[^\s](?: ?[^\s])* ?$/u.test(value);

export function hasUnsafeText(value, allowContextWhitespace = false) {
  const scalars = [...value];
  return scalars.some((character, index) => {
    // Preserve Persian half-spaces only in the shared, exact orthographic
    // contract. No bidi marks or arbitrary invisible formatting are allowed.
    if (character === '\u200c') return !arabicLetter(scalars[index - 1]) || !arabicLetter(scalars[index + 1]);
    if (allowContextWhitespace && (character === '\n' || character === '\t')) return false;
    return /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}\p{Cs}]/u.test(character);
  });
}

// A correction owns one exact word of 2-32 characters with case (Unicode
// Lowercase or Uppercase, protocol/spelling-replacement-fixtures.json),
// including at most
// its newly typed space. It never broadens into an arbitrary document range.
export function validCorrection(before, original, replacement) {
  if (typeof before !== 'string' || typeof original !== 'string' || typeof replacement !== 'string' ||
      !/^[\p{Lowercase}\p{Uppercase}]{2,32} ?$/u.test(original) || !/^[\p{Lowercase}\p{Uppercase}]{2,32} ?$/u.test(replacement) ||
      original === replacement || original.endsWith(' ') !== replacement.endsWith(' ') ||
      !before.endsWith(original)) return false;
  const start = before.length - original.length;
  return start === 0 || /\s/u.test(before[start - 1]);
}
