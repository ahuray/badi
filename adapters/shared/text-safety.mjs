const arabicLetter = value => value !== undefined && /[\u0620-\u063f\u0641-\u064a\u066e-\u066f\u0671-\u06d3\u06d5\u06e5-\u06e6\u06ee-\u06ef\u06fa-\u06fc\u06ff]/u.test(value);

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

// A correction owns one exact English suffix, including at most its newly
// typed space. It never broadens into an arbitrary document range.
export function validCorrection(before, original, replacement) {
  if (typeof before !== 'string' || typeof original !== 'string' || typeof replacement !== 'string' ||
      !/^[a-z]{3,24} ?$/u.test(original) || !/^[a-z]{3,24} ?$/u.test(replacement) ||
      /[^a-z ]/u.test(original + replacement) ||
      original === replacement || original.endsWith(' ') !== replacement.endsWith(' ') ||
      !before.endsWith(original)) return false;
  const start = before.length - original.length;
  return start === 0 || /\s/u.test(before[start - 1]);
}
