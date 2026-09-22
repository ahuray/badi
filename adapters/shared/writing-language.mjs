// Language is a quality hint from an already authorized context window. It
// never grants field access. Latin English/German cannot be inferred reliably
// from a short prefix, so retain the explicit language or application locale.
export function writingLanguage(before, preferred) {
  let language;
  try {
    if (typeof preferred === 'string' && preferred.length <= 35) [language] = Intl.getCanonicalLocales(preferred);
  } catch { return undefined; }
  const primary = language?.split('-')[0];
  const lastLetter = before.match(/\p{L}(?=[^\p{L}]*$)/u)?.[0];
  if ((primary === undefined || ['en', 'de', 'fa'].includes(primary)) &&
      lastLetter && /\p{Script=Arabic}/u.test(lastLetter)) return 'fa';
  return language;
}
