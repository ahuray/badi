const SCHEMA = 'badi.context-lookup.request.v1';
const MAX_RECORDS = 50;

export function lookupInput({ before, language, context = '', style = '' }, id) {
  const bounded = (value, limit) => typeof value === 'string' && value.isWellFormed() && [...value].length <= limit;
  if (!bounded(before, 2048) || !before.trim() || !['en', 'de', 'fa'].includes(language)
    || !bounded(context, 4096) || !bounded(style, 8256)) {
    throw new Error('Use an English, German or Persian draft of up to 2,048 characters and context of up to 4,096 characters.');
  }
  const examples = style.split(/\n\s*\n/u).filter(value => value.trim());
  if (examples.length > 8 || examples.some(value => !bounded(value, 1024))) {
    throw new Error('Use up to eight writing examples, each no longer than 1,024 characters.');
  }
  // Only these fields can reach the lookup; expected answers remain local.
  return { schema: SCHEMA, id, before, language, context, style_examples: examples };
}

export function lookupParts(input, result) {
  if (result.outcome === 'abstention') return null;
  const stem = input.before.match(/[\p{Alphabetic}\p{Mark}\u200c]+$/u)?.[0];
  if (result.outcome !== 'suggestion' || typeof result.text !== 'string' || !result.text
    || !result.text.isWellFormed() || /\s/u.test(result.text)
    || !stem || result.matched_word !== stem + result.text || result.matched_candidate_count !== 1) {
    throw new Error('The completion does not match the submitted partial word.');
  }
  return { before: input.before, suffix: result.text };
}

export function attachLookupUI(document, fetchRequest = globalThis.fetch) {
  const $ = id => document.getElementById(id);
  const headers = { 'Content-Type': 'application/json', 'X-Badi-Lab-Token': document.querySelector('meta[name="badi-lab-token"]').content };
  const records = [];
  let active = null; let resetting = null; let resetFailed = false; let generation = 0;
  const fields = ['lookup-before', 'lookup-context', 'lookup-style', 'lookup-expected'];
  const node = (tag, text, className) => {
    const value = document.createElement(tag);
    if (text !== undefined) value.textContent = text;
    if (className) value.className = className;
    return value;
  };
  const status = (text, error = false) => {
    $('lookup-status').textContent = text;
    $('lookup-status').classList.toggle('lookup-error', error);
  };
  const controls = () => {
    $('lookup-check').disabled = Boolean(active || resetting || resetFailed);
    $('lookup-cancel').hidden = !active;
    $('lookup-clear').disabled = Boolean(resetting);
    $('lookup-export').disabled = !records.length && fields.every(id => !$(id).value);
    $('lookup-count').textContent = String(records.length);
  };
  const empty = message => $('lookup-preview').replaceChildren(node('p', message, 'empty'));
  const reason = result => ({
    no_eligible_stem: 'Leave at least three letters of an unfinished word after a space.',
    no_matching_word: 'No complete matching word was found in the supplied text.',
    ambiguous_matches: 'More than one word matches, so no completion is proposed.',
    unsupported_suffix: 'The matching word could not be safely joined to this draft.',
  })[result.reason] ?? 'No completion was proposed. Inspect the result for its reason.';
  const preview = record => {
    const parts = lookupParts(record.input, record.response.result);
    if (!parts) return node('p', reason(record.response.result), 'diagnostic');
    const paragraph = node('p', undefined, 'prediction'); paragraph.dir = 'auto';
    paragraph.append(node('span', parts.before, 'draft'), node('span', parts.suffix, 'lookup-suffix'));
    return paragraph;
  };
  function recordResult(record) {
    if (!records.length) $('lookup-results').replaceChildren();
    records.push(record);
    const article = node('article', undefined, 'result');
    const heading = node('div', undefined, 'lookup-result-head');
    heading.append(node('h3', `Check ${records.length} · ${record.input.language.toUpperCase()}`),
      node('p', `${Math.round(record.delivery_ms)} ms to receive the result`));
    article.append(heading, preview(record));
    article.append(node('p', record.expected_text
      ? `Expected suffix: ${record.expected_text} · ${record.response.result.text === record.expected_text ? 'Exact match.' : 'Does not match.'}`
      : 'No expected suffix supplied. Judge the completion yourself.', 'diagnostic'));
    const review = node('div', undefined, 'review lookup-review'); review.append(node('span', 'Your judgment:'));
    for (const judgment of ['Useful', 'Acceptable alternative', 'Unhelpful']) {
      const button = node('button', judgment, 'secondary'); button.type = 'button';
      button.setAttribute('aria-pressed', 'false');
      button.onclick = () => {
        record.review = { judgment, reviewer: 'user', at: new Date().toISOString() };
        for (const other of review.querySelectorAll('button')) other.setAttribute('aria-pressed', String(other === button));
      };
      review.append(button);
    }
    article.append(review);
    const details = node('details'); details.append(node('summary', 'Inspect result and timing'));
    details.append(node('pre', JSON.stringify(record.response, null, 2), 'trace'));
    article.append(details); $('lookup-results').prepend(article);
    $('lookup-preview').replaceChildren(preview(record));
  }
  async function reset(message, clear = false) {
    generation++; active?.abort(); active = null;
    if (clear) {
      for (const id of [...fields, 'lookup-example']) $(id).value = '';
      records.length = 0;
      $('lookup-results').replaceChildren(node('p', 'No checks yet.', 'hint'));
    }
    empty(clear ? 'Your word tests are cleared.' : 'Run a new check for the edited draft.');
    status('Stopping the current lookup…');
    if (!resetting) resetting = (async () => {
      try {
        const response = await fetchRequest('/api/context-lookup/reset', { method: 'POST', headers, body: '{}' });
        if (!response.ok) throw new Error('Could not confirm cleanup. Clear the word tests again before retrying.');
        resetFailed = false;
      } catch (error) { resetFailed = true; throw error; }
    })();
    const pending = resetting; controls();
    try { await pending; status(message); }
    catch (error) { status(error.message, true); }
    finally { if (resetting === pending) resetting = null; controls(); }
  }
  $('lookup-form').onsubmit = async event => {
    event.preventDefault();
    if (active || resetting || resetFailed) return;
    if (records.length >= MAX_RECORDS) return status('Export or clear these 50 tests before starting another.', true);
    let input;
    try { input = lookupInput({ before: $('lookup-before').value, language: $('lookup-language').value,
      context: $('lookup-context').value, style: $('lookup-style').value }, crypto.randomUUID()); }
    catch (error) { return status(error.message, true); }
    const expected = $('lookup-expected').value;
    const revision = ++generation;
    const controller = new AbortController(); active = controller; controls();
    status('Looking for a matching word…'); empty('Checking…');
    const started = performance.now();
    try {
      const response = await fetchRequest('/api/context-lookup', { method: 'POST', headers,
        body: JSON.stringify(input), signal: controller.signal });
      const body = await response.json();
      const delivery = performance.now() - started;
      if (revision !== generation || active !== controller) return;
      if (response.status === 403) throw new Error('The Lab connection changed. Export your tests, then reload to reconnect.');
      if (response.status === 409) throw new Error('Another writing test or reset is active. Finish or cancel it, then try again.');
      if (!response.ok) throw new Error(({
        context_lookup_invalid_input: 'Check the draft, context and language. Use complete Unicode text within the stated limits.',
        context_lookup_worker_unavailable: 'The word-completion worker is unavailable. Restart the Lab with npm run writing:lab to build it.',
        context_lookup_deadline: 'The lookup exceeded its time limit. Try a fresh check.',
        context_lookup_cleanup_unverified: 'Worker cleanup could not be confirmed. Export your tests, then restart the Lab server.',
      })[body.error] ?? 'The lookup could not complete. Clear the word tests before retrying.');
      if (body.schema !== 'badi.context-lookup.response.v1' || body.result?.id !== input.id
        || body.result?.schema !== 'badi.context-lookup.result.v1' || body.cleanup?.reaped !== true) {
        throw new Error('The lookup returned an invalid result.');
      }
      lookupParts(input, body.result);
      recordResult({ input, expected_text: expected, response: body, delivery_ms: delivery, checked_at: new Date().toISOString() });
      status(body.result.outcome === 'suggestion' ? 'Completion ready. Your draft is unchanged.' : reason(body.result));
    } catch (error) {
      if (revision === generation && active === controller) { status(error.message, true); empty('No completion is available for this check.'); }
    } finally { if (active === controller) active = null; controls(); }
  };
  const changed = () => {
    empty('Run a new check for the edited draft.');
    if (active) void reset('The test changed. Ready for a fresh check.');
    controls();
  };
  for (const id of fields) $(id).addEventListener('input', changed);
  $('lookup-language').addEventListener('change', changed);
  $('lookup-cancel').onclick = () => void reset('Word completion cancelled.');
  $('lookup-clear').onclick = () => void reset('Word tests cleared.', true);
  $('lookup-example').onchange = () => {
    const examples = {
      en: { before: 'Please send the updated sched', context: 'The schedule is ready.', expected: 'ule' },
      de: { before: 'Wir treffen uns am Bahnhof. Treffpunkt ist der Bahn', context: '', expected: 'hof' },
      fa: { before: 'قرار ما در کتابخانه است. می‌روم به کتابخ', context: '', expected: 'انه' },
    };
    const language = $('lookup-example').value; const value = examples[language];
    if (!value) return;
    $('lookup-language').value = language; $('lookup-before').value = value.before;
    $('lookup-context').value = value.context; $('lookup-style').value = ''; $('lookup-expected').value = value.expected;
    changed();
  };
  $('lookup-export').onclick = () => {
    const draft = { before: $('lookup-before').value, language: $('lookup-language').value,
      context: $('lookup-context').value, style: $('lookup-style').value, expected_text: $('lookup-expected').value };
    const data = { schema: 'badi.context-lookup.session.v1', exported_at: new Date().toISOString(), draft, records };
    const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }));
    const link = node('a'); link.href = url; link.download = 'badi-word-tests.json'; link.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  };
  controls();
}

if (typeof document !== 'undefined') attachLookupUI(document);
