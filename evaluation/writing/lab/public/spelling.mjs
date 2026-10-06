import { attachLabTabs } from './tabs.mjs';

const MAX_RECORDS = 50;

export function spellingInput({ before, language, protectedWords = '' }) {
  if (typeof before !== 'string' || !before.isWellFormed() || [...before].length > 2048
    || !['de', 'fa'].includes(language) || typeof protectedWords !== 'string') {
    throw new Error('Use a German or Persian draft of at most 2,048 characters.');
  }
  const words = protectedWords.split(/\r?\n/u).map(word => word.trim()).filter(Boolean);
  if (words.length > 32 || words.some(word => [...word].length > 24 || /\s/u.test(word))) {
    throw new Error('Enter up to 32 words to keep, one word per line, at most 24 characters each.');
  }
  // The expected answer is deliberately absent from the engine request.
  return { before, language, protected_words: words };
}

export function correctionParts(input, result) {
  if (result.outcome !== 'suggestion') return null;
  const original = result.replace_before;
  const corrected = result.text;
  if (typeof original !== 'string' || typeof corrected !== 'string' || original === corrected
    || !original.endsWith(' ') || !corrected.endsWith(' ') || !input.before.endsWith(original)) {
    throw new Error('The correction does not match the submitted draft.');
  }
  const oldWord = original.slice(0, -1);
  const newWord = corrected.slice(0, -1);
  const start = input.before.length - original.length;
  if (!oldWord || !newWord || /\s/u.test(oldWord + newWord)
    || (start > 0 && !/\s/u.test(input.before[start - 1]))) {
    throw new Error('The correction does not match one complete word.');
  }
  return { prefix: input.before.slice(0, start), original: oldWord, corrected: newWord };
}

export function spellingAgreement(input, result, expectedWord) {
  const parts = correctionParts(input, result);
  return expectedWord ? { scorable: true, exact_match: parts?.corrected === expectedWord }
    : { scorable: false, exact_match: null };
}

export function attachSpellingUI(document, fetchRequest = globalThis.fetch) {
  const $ = id => document.getElementById(id);
  const token = document.querySelector('meta[name="badi-lab-token"]').content;
  const headers = { 'Content-Type': 'application/json', 'X-Badi-Lab-Token': token };
  const records = [];
  let generation = 0;
  let active = null;
  let resetting = null;
  let resetFailed = false;
  let configured = null;
  const node = (tag, text, className) => {
    const element = document.createElement(tag);
    if (text !== undefined) element.textContent = text;
    if (className) element.className = className;
    return element;
  };
  const status = (message, error = false) => {
    $('spelling-status').textContent = message;
    $('spelling-status').classList.toggle('spelling-error', error);
  };
  const refreshControls = () => {
    $('spelling-check').disabled = Boolean(active || resetting || resetFailed);
    $('spelling-cancel').hidden = !active;
    $('spelling-clear').disabled = Boolean(resetting);
    $('spelling-export').disabled = records.length === 0
      && ['spelling-before', 'spelling-expected', 'spelling-protected'].every(id => !$(id).value);
    $('spelling-count').textContent = String(records.length);
  };
  const emptyPreview = message => $('spelling-preview').replaceChildren(node('p', message, 'empty'));
  const errorMessage = (code, fallback) => ({
    spelling_not_configured: 'Spelling is not configured for this language. Start the Lab with its local dictionary manifest.',
    spelling_engine_unavailable: 'The dictionary could not start. Check the Lab server and try again.',
    spelling_busy: 'A writing test is running. Finish or cancel it before checking spelling.',
    spelling_invalid_input: 'Check the draft, language and words to keep, then try again.',
    spelling_cancelled: 'This check was cancelled. Run a new check when ready.',
    spelling_cleanup_unverified: 'Dictionary cleanup could not be confirmed. Export your tests, then restart the Lab server.',
  })[code] ?? fallback;
  const reasonMessage = result => {
    const reasons = {
      dictionary_accepts_original: 'The dictionary accepts this word; no change is proposed.',
      protected_word: 'This word is on your keep-unchanged list.',
      no_completed_word: 'Finish a supported word with one space before checking it.',
      ambiguous_candidates: 'Several corrections remain possible, so no single change is proposed.',
      no_admissible_candidate: 'No correction passed the spelling checks.',
      candidate_not_accepted: 'The proposed word did not pass the final dictionary check.',
    };
    if (result.outcome === 'deadline') return 'The dictionary exceeded this check’s time limit.';
    return reasons[result.reason] ?? 'No correction was proposed. The result details explain why.';
  };
  const preview = record => {
    const { input, response } = record;
    const result = response.result;
    const parts = correctionParts(input, result);
    if (!parts) return node('p', reasonMessage(result), 'diagnostic');
    const paragraph = node('p', undefined, 'prediction'); paragraph.dir = 'auto';
    paragraph.append(node('span', parts.prefix, 'draft'));
    const pair = node('span', undefined, 'spelling-word-pair');
    const original = node('del', parts.original, 'spelling-before-word'); original.dir = 'auto';
    const corrected = node('ins', parts.corrected, 'spelling-after-word'); corrected.dir = 'auto';
    pair.append(original, corrected);
    paragraph.append(pair, node('span', ' '));
    return paragraph;
  };
  function appendRecord(record) {
    if (!records.length) $('spelling-results').replaceChildren();
    records.push(record);
    const article = node('article', undefined, 'result');
    const heading = node('div', undefined, 'spelling-result-head');
    const elapsed = Number.isFinite(record.delivery_ms) ? `${Math.round(record.delivery_ms)} ms to receive the result` : 'Delivery timing unavailable';
    heading.append(node('h3', `Check ${records.length} · ${record.input.language.toUpperCase()}`), node('p', elapsed));
    article.append(heading, preview(record));
    const agreement = spellingAgreement(record.input, record.response.result, record.expected_word);
    article.append(node('p', agreement.scorable
      ? `Expected: ${record.expected_word} · ${agreement.exact_match ? 'Matches your expected word.' : 'Does not match your expected word.'}`
      : 'No expected word supplied. Judge the proposed correction yourself.', 'diagnostic'));
    const review = node('div', undefined, 'review spelling-judgment');
    review.append(node('span', 'Your judgment:'));
    for (const judgment of ['Useful', 'Acceptable alternative', 'Unhelpful']) {
      const button = node('button', judgment, 'secondary'); button.type = 'button';
      button.setAttribute('aria-pressed', 'false');
      button.onclick = () => {
        record.review = { judgment, at: new Date().toISOString(), reviewer: 'user' };
        for (const other of review.querySelectorAll('button')) other.setAttribute('aria-pressed', String(other === button));
      };
      review.append(button);
    }
    article.append(review);
    const details = node('details'); details.append(node('summary', 'Inspect result and timing'));
    details.append(node('pre', JSON.stringify(record.response, null, 2), 'trace')); article.append(details);
    $('spelling-results').prepend(article);
    $('spelling-preview').replaceChildren(preview(record));
  }

  async function resetSession(message, clear = false) {
    generation++;
    active?.abort(); active = null;
    if (clear) {
      records.length = 0;
      for (const id of ['spelling-before', 'spelling-expected', 'spelling-protected', 'spelling-example']) $(id).value = '';
      $('spelling-results').replaceChildren(node('p', 'No checks yet.', 'hint'));
    }
    emptyPreview(clear ? 'Your spelling session is cleared.' : 'Run a new check for the current draft.');
    status('Stopping the dictionary session…');
    if (!resetting) {
      resetting = (async () => {
        try {
          const response = await fetchRequest('/api/spelling/reset', { method: 'POST', headers, body: '{}' });
          if (!response.ok) throw new Error('Could not confirm dictionary cleanup. Try clearing the spelling session again.');
          resetFailed = false;
        } catch (error) { resetFailed = true; throw error; }
      })();
    }
    const pending = resetting;
    refreshControls();
    try { await pending; status(message); }
    catch (error) { status(error.message, true); }
    finally { if (resetting === pending) resetting = null; refreshControls(); }
  }

  $('spelling-form').onsubmit = async event => {
    event.preventDefault();
    if (active || resetting || resetFailed) return;
    if (records.length >= MAX_RECORDS) return status('Export or clear these 50 spelling checks before starting another.', true);
    let input;
    try { input = spellingInput({ before: $('spelling-before').value, language: $('spelling-language').value, protectedWords: $('spelling-protected').value }); }
    catch (error) { return status(error.message, true); }
    if (configured && !configured.includes(input.language)) return status(errorMessage('spelling_not_configured'), true);
    const expectedWord = $('spelling-expected').value.trim();
    const snapshot = ++generation;
    const controller = new AbortController(); active = controller;
    refreshControls(); status('Checking the finished word…'); emptyPreview('Checking…');
    const deliveryStarted = performance.now();
    try {
      const response = await fetchRequest('/api/spelling', { method: 'POST', headers, body: JSON.stringify(input), signal: controller.signal });
      const body = await response.json();
      const deliveryMs = performance.now() - deliveryStarted;
      if (snapshot !== generation || active !== controller) return;
      if (response.status === 403) throw new Error('The Lab server connection changed. Export your tests, then reload this page to reconnect.');
      if (!response.ok) throw new Error(errorMessage(body.error, 'The spelling check could not complete. Clear the session and try again.'));
      if (body.schema !== 'badi.spelling-lab.response.v1' || !body.result
        || !['suggestion', 'abstention', 'deadline'].includes(body.result.outcome)) throw new Error('The dictionary returned an invalid response.');
      correctionParts(input, body.result);
      appendRecord({ input, expected_word: expectedWord, response: body, delivery_ms: deliveryMs, checked_at: new Date().toISOString() });
      status(body.result.outcome === 'suggestion' ? 'Correction ready. Your draft is unchanged.' : reasonMessage(body.result));
    } catch (error) {
      if (snapshot === generation && active === controller) {
        status(error.name === 'AbortError' ? 'Check cancelled.' : error.message, true);
        emptyPreview('No correction is available for this check.');
      }
    } finally { if (active === controller) active = null; refreshControls(); }
  };
  $('spelling-cancel').onclick = () => void resetSession('Check cancelled.');
  $('spelling-clear').onclick = () => void resetSession('Spelling session cleared.', true);
  const changed = () => {
    emptyPreview('Check the edited draft to see its correction.');
    if (active) void resetSession('The test changed. Run a new check when ready.');
    refreshControls();
  };
  for (const id of ['spelling-before', 'spelling-expected', 'spelling-protected']) $(id).addEventListener('input', changed);
  $('spelling-language').addEventListener('change', changed);
  $('spelling-example').onchange = () => {
    const examples = {
      de: { language: 'de', before: 'Ich gehe heute in die Bibliotek ', expected: 'Bibliothek', protected: '' },
      fa: { language: 'fa', before: 'من با دوچرجه ', expected: 'دوچرخه', protected: '' },
      protected: { language: 'fa', before: 'گزارش برای آترین ', expected: '', protected: 'آترین' },
    };
    const example = examples[$('spelling-example').value];
    if (!example) return;
    changed();
    $('spelling-language').value = example.language; $('spelling-before').value = example.before;
    $('spelling-expected').value = example.expected; $('spelling-protected').value = example.protected;
  };
  $('spelling-export').onclick = () => {
    const draft = { before: $('spelling-before').value, language: $('spelling-language').value,
      protected_words_text: $('spelling-protected').value, expected_word: $('spelling-expected').value };
    const data = { schema: 'badi.spelling-lab.session.v1', exported_at: new Date().toISOString(), draft, records };
    const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }));
    const link = node('a'); link.href = url; link.download = 'badi-spelling-tests.json'; link.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  };

  attachLabTabs(document);
  void (async () => {
    try {
      const response = await fetchRequest('/api/spelling/status', { headers });
      const body = await response.json();
      if (!response.ok || !Array.isArray(body.configured_languages)) throw new Error();
      configured = body.configured_languages.filter(language => ['de', 'fa'].includes(language));
      if (!active && !resetting && !records.length && generation === 0) status(configured.length
        ? 'Ready. The dictionary starts when you check a word.'
        : 'Spelling needs local dictionary manifests. Start the Lab with its spelling configuration.');
    } catch { if (!active && !resetting && generation === 0) status('Spelling status is unavailable. Check the Lab server configuration.', true); }
  })();
  refreshControls();
}

if (typeof document !== 'undefined') attachSpellingUI(document);
