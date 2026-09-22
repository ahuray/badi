import { mountDiscovery } from './discovery.mjs';
import { mountQualification } from './qualification.mjs';
import { confidenceCoverage } from './confidence.mjs';
const $ = id => document.getElementById(id);
const token = document.querySelector('meta[name="badi-lab-token"]').content;
const headers = { 'Content-Type': 'application/json', 'X-Badi-Lab-Token': token };
const names = { production_baseline: 'Current Badi logic', production_boundary: 'Isolated boundary fix', context: 'Full context', context_confidence: 'Full context + token confidence', instructed: 'Context + style', healed: 'Word-boundary experiment', instructed_healed: 'Instructions + word boundary', instructed_word: 'One complete word', healed_attested: 'Complete words from context', native_instructed: 'Selected model instructions' };
const fixedProductionMode = mode => mode === 'production_baseline' || mode === 'production_boundary';
const reasonLabels = {
  constrained_complete_word: 'The model finished one word with a separator and a completion response. Its decoding was constrained to this shape.',
  word_terminal_missing: 'The model did not confirm completion before the deadline.',
  word_separator_missing: 'The model stopped without the required word-ending space.',
  word_spacing: 'The proposed spacing does not match this word boundary.',
  word_grammar_shape: 'The response did not match the one-word decoding constraint.',
  word_count: 'The response contains no usable word.',
  unsupported_compound: 'This one-word experiment cannot return a compound containing multiple Unicode words.',
  production_abstention_or_budget: 'The current provider returned no usable continuation. Its response does not distinguish an abstention from an exhausted time budget.',
  no_complete_word: 'The model did not produce a complete word.',
  token_preflight_deadline: 'The time budget expired while checking the prompt.',
  context_overflow: 'The supplied text exceeds the model context. Shorten the draft or examples.',
  runtime_unavailable: 'The local model could not complete this request.',
  unsupported_language: 'This experiment currently supports English, German and Persian.',
  invalid_request: 'The input exceeds the supported limits or contains unsupported formatting.',
  invalid_model_output: 'The model returned an invalid response.',
  attested_context_word_suffix: 'The completed word occurs exactly in your supplied context. This does not verify its spelling.',
  attested_style_word_suffix: 'The completed word occurs exactly in your supplied writing examples. This does not verify its spelling.',
};
let cases = [];
let session = null;
let controller = null;
let editedDuringRun = false;
let resetting = false;
let importRevision = 0;
const examples = {
  'context-en': { id: 'weekend-plan', language: 'en', prefix: 'We chose the Black Forest for our weekend trip. We will spend Saturday hiking in the ', expected: ['Black Forest'], context: '', style: '' },
  'context-de': { id: 'wochenende', language: 'de', prefix: 'Für unseren Ausflug haben wir den Schwarzwald gewählt. Am Samstag wandern wir im ', expected: ['Schwarzwald'], context: '', style: '' },
  'context-fa': { id: 'weekend-fa', language: 'fa', prefix: 'برای سفر آخر هفته شهر شیراز را انتخاب کردیم. روز جمعه به ', expected: ['شیراز'], context: '', style: '' },
  partial: { id: 'partial-word', language: 'en', prefix: 'Please review the docum', expected: ['entation', 'ent'], context: '', style: '' },
  style: { id: 'email-signoff', language: 'en', prefix: 'I have attached the revised itinerary.\n\n', expected: ['Cheers,'], context: 'A short email to a colleague about a walking tour.', style: 'The draft is ready for your review.\n\nCheers,\nAlex\n\nThe changes are in the shared document.\n\nCheers,\nAlex' },
};

function editorCase() {
  return { id: $('case-name').value, language: $('language').value, prefix: $('prefix').value,
    expected: $('expected').value.split('\n').filter(value => value.length), context: $('context').value, style: $('style').value };
}
function fillEditor(value) {
  for (const [id, key] of [['case-name', 'id'], ['language', 'language'], ['prefix', 'prefix'], ['context', 'context'], ['style', 'style']]) $(id).value = value[key] ?? '';
  $('expected').value = value.expected.join('\n');
}
function setStatus(value) { $('status').textContent = value; }
function node(tag, text, className) { const value = document.createElement(tag); if (text !== undefined) value.textContent = text; if (className) value.className = className; return value; }
function renderCases() {
  $('case-count').textContent = String(cases.length);
  $('cases').replaceChildren();
  for (const item of cases) {
    const row = node('div', undefined, 'case-row');
    const edit = node('button', `${item.id} · ${item.language}${item.trace_id ? ` · trace ${item.step}` : ''}`, 'quiet');
    edit.onclick = () => fillEditor(item);
    const remove = node('button', 'Remove', 'quiet');
    remove.onclick = () => { cases = cases.filter(value => value.id !== item.id); renderCases(); };
    row.append(edit, remove); $('cases').append(row);
  }
  if (!cases.length) $('cases').append(node('p', 'The comparison will use the current editor.', 'hint'));
}
function configs() {
  return [...document.querySelectorAll('input[name="mode"]:checked')].map(input => ({ id: input.value, mode: input.value,
    budget_ms: fixedProductionMode(input.value) ? 550 : Number($('budget').value),
    max_tokens: fixedProductionMode(input.value) ? 8 : Number($('tokens').value),
    cache_prompt: fixedProductionMode(input.value) || $('cache').checked,
    temperature: fixedProductionMode(input.value) ? 0 : Number($('temperature').value), seed: 42 }));
}
function renderRecord(record, index) {
  const { case: testCase, config, result, score } = record;
  const article = node('article', undefined, 'result');
  const top = node('div', undefined, 'result-top');
  const title = node('div', `${testCase.id} · ${names[config.mode]}`, 'result-title');
  title.append(node('span', result.outcome, `badge${result.outcome === 'error' ? ' warning' : ''}`));
  const latency = Number.isFinite(result.latency_ms) ? `${Math.round(result.latency_ms)} ms` : 'timing unavailable';
  top.append(title, node('span', `${testCase.language.toUpperCase()} · ${latency}`, 'result-meta'));
  const preview = node('div', undefined, 'prediction'); preview.dir = 'auto';
  const replaced = result.replace_before;
  const before = typeof replaced === 'string' && testCase.prefix.endsWith(replaced) ? testCase.prefix.slice(0, -replaced.length) : testCase.prefix;
  preview.append(node('span', before, 'draft'));
  preview.append(node('span', result.text ?? '  [No suggestion]', result.text ? 'completion' : 'warning'));
  const reference = testCase.expected.length ? `Expected addition: ${testCase.expected.map(value => JSON.stringify(value)).join(' or ')}` : 'No reference supplied. Review the continuation below.';
  const diagnostic = node('p', `${reference}${replaced ? ' · Spelling replacement; continuation agreement does not evaluate correction.' : ''}`, 'diagnostic');
  const review = node('div', undefined, 'review'); review.append(node('span', 'Your judgment:'));
  for (const value of ['Useful', 'Acceptable alternative', 'Unhelpful', 'Harmful']) {
    const button = node('button', value, 'secondary'); button.setAttribute('aria-pressed', 'false');
    button.onclick = () => { session.records[index].review = { judgment: value, reviewer: 'user', at: new Date().toISOString() }; for (const other of review.querySelectorAll('button')) other.setAttribute('aria-pressed', String(other === button)); renderSummary(); };
    review.append(button);
  }
  const details = node('details'); details.append(node('summary', 'Inspect prompt, output and measurements'));
  details.append(node('pre', JSON.stringify({ score, result }, null, 2), 'trace'));
  article.append(top, preview, diagnostic);
  if (result.reason && !result.text) article.append(node('p', reasonLabels[result.reason] ?? 'No usable prediction was delivered. Inspect the measurements for the exact reason.', 'diagnostic warning'));
  if (Number.isFinite(result.first_word_ms)) article.append(node('p', `First complete word observed at ${Math.round(result.first_word_ms)} ms. This diagnostic timestamp precedes the completed result shown above.`, 'diagnostic'));
  if (Number.isFinite(result.candidate_mean_token_logprob)) article.append(node('p', `Mean candidate token log probability: ${result.candidate_mean_token_logprob.toFixed(3)} across ${result.candidate_logprob_token_count} tokens. This is an uncalibrated model feature, not a correctness score.`, 'diagnostic'));
  article.append(node('p', 'Useful confirms substantive value and correctness of the entire addition, including its tail. Acceptable alternative alone does not earn useful credit.', 'hint'), review);
  article.append(details); $('results').append(article);
}
function renderSummary() {
  const confidenceOpen = $('summary').querySelector?.('details')?.open ?? false;
  $('summary').replaceChildren();
  const grid = node('div', undefined, 'summary-grid');
  for (const config of session.configs) {
    const records = session.records.filter(record => record.config.id === config.id);
    const shown = records.filter(record => record.result.outcome === 'suggestion');
    const scored = records.filter(record => record.score.scorable);
    const exact = scored.filter(record => record.score.raw_exact_match);
    const first = scored.filter(record => record.score.first_word_match);
    const item = node('div', undefined, 'summary-item'); item.append(node('strong', names[config.mode]));
    item.append(node('div', `${shown.length} suggestions / ${records.length} completed requests`));
    item.append(node('div', `${exact.length} exact reference matches / ${scored.length} scored requests`));
    item.append(node('div', `${first.length} first-word matches / ${scored.length} scored requests`));
    item.append(node('div', `${records.filter(record => record.result.outcome === 'error').length} errors · ${records.filter(record => record.result.outcome === 'deadline').length} deadlines`));
    grid.append(item);
  }
  $('summary').append(grid);
  const groups = confidenceCoverage(session.records);
  if (groups.length) {
    const details = node('details'); details.open = confidenceOpen;
    details.append(node('summary', 'Experimental confidence and coverage'));
    details.append(node('p', 'Review the entire suggestion, including its tail. Scores rank only aligned, on-time suggestions. Missing scores, late results and abstentions stay in the request count. No threshold is selected.', 'hint'));
    for (const group of groups) {
      const section = node('section', undefined, 'summary-item confidence-group');
      section.append(node('strong', `${group.label}: ${group.scored_suggestions}/${group.requests} requests have an aligned on-time score`));
      section.append(node('p', `${group.missing_scores} on-time suggestions lack a score · ${group.reviewed_scores}/${group.scored_suggestions} scored suggestions reviewed`, 'diagnostic'));
      const table = node('table', undefined, 'confidence-table');
      const header = node('tr');
      for (const label of ['Minimum log score', 'Coverage', 'Useful / harmful', 'Useful precision']) header.append(node('th', label));
      table.append(header);
      for (const point of group.points) {
        const row = node('tr');
        for (const value of [point.threshold.toFixed(3), `${point.retained}/${group.requests}`,
          `${point.useful}/${point.harmful}${point.reviewed < point.retained ? ' · review incomplete' : ''}`,
          point.precision === null ? 'Review pending' : `${Math.round(point.precision * 100)}%`]) row.append(node('td', value));
        table.append(row);
      }
      section.append(table); details.append(section);
    }
    $('summary').append(details);
  }
}

$('run').onclick = async () => {
  if (controller || resetting) return;
  const selected = configs();
  if (!selected.length) return setStatus('Select a configuration first.');
  controller = new AbortController();
  const runController = controller;
  editedDuringRun = false;
  const suite = { schema: 'badi.prediction-suite.v1', name: 'My prediction tests', cases: structuredClone(cases.length ? cases : [editorCase()]) };
  session = { schema: 'badi.prediction-lab.session.v1', suite, configs: selected, seed: Number($('seed').value), records: [], complete: false };
  qualificationUI?.clear();
  const runSession = session;
  $('results').replaceChildren(); $('summary').replaceChildren(); $('run').disabled = true; $('cancel').hidden = false; $('export').disabled = true;
  $('result-note').textContent = 'Results belong to the exact test snapshot submitted below. Model timing excludes loading. Review alternatives before treating reference disagreement as a failure.';
  setStatus('Loading the verified local model…');
  try {
    const confirmation = $('confirmation-untouched')?.checked ? { untouched: true, review_protocol: $('confirmation-protocol').value } : undefined;
    const response = await fetch('/api/run', { method: 'POST', headers, body: JSON.stringify({ suite, configs: selected, seed: session.seed, ...(confirmation ? { confirmation } : {}) }), signal: controller.signal });
    if (!response.ok) throw new Error((await response.json()).error);
    const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
    let buffer = '';
    const event = value => {
      if (session !== runSession) return;
      if (value.type === 'plan') { session.plan = value; }
      if (value.type === 'cleanup') { (session.cleanups ??= []).push(value.cleanup); }
      if (value.type === 'progress') setStatus(`Running ${value.completed + 1} of ${value.total}: ${value.case_id} · ${names[value.config_id] ?? value.config_id}`);
      if (value.type === 'record') { const record = { ...value.record, review_target: value.review_target }; session.records.push(record); renderRecord(record, session.records.length - 1); renderSummary(); $('export').disabled = false; }
      if (value.type === 'complete') { session.summaries = value.summaries; session.complete = true; setStatus(`Finished ${session.records.length} ${session.records.length === 1 ? 'prediction' : 'predictions'}.${editedDuringRun ? ' The editor changed during this run; results use the submitted snapshot.' : ''}`); }
      if (value.type === 'error' || value.type === 'cancelled') throw new Error(value.error);
    };
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) break;
      buffer += chunk.value;
      let newline;
      while ((newline = buffer.indexOf('\n')) >= 0) { const line = buffer.slice(0, newline); buffer = buffer.slice(newline + 1); if (line) event(JSON.parse(line)); }
    }
    if (session === runSession && !session.complete) throw new Error('The run ended before completion. Partial results are retained.');
  } catch (error) { if (session === runSession) setStatus(error.name === 'AbortError' ? 'Comparison cancelled. Partial results remain available.' : error.message); }
  finally { if (controller === runController) controller = null; $('run').disabled = resetting; $('cancel').hidden = true; if ($('confirmation-untouched')) $('confirmation-untouched').checked = false; if (session === runSession && session.complete) void qualificationUI?.refresh(); }
};
$('cancel').onclick = () => { controller?.abort(); };
$('example').onchange = () => {
  if (examples[$('example').value]) fillEditor(examples[$('example').value]);
  if ($('example').value === 'style') setStatus('This example tests reuse of a supplied sign-off. It does not demonstrate general writing-style learning.');
};
$('add-case').onclick = () => {
  const item = editorCase();
  if (!item.id || !item.prefix) return setStatus('Add a name and draft first.');
  const index = cases.findIndex(value => value.id === item.id);
  if (index >= 0) cases[index] = item; else cases.push(item);
  renderCases(); setStatus(`${item.id} ${index >= 0 ? 'updated' : 'added'} in the test set.`);
};
$('use-editor').onclick = () => { cases = []; renderCases(); };
$('clear-editor').onclick = () => fillEditor({ id: 'new-case', language: 'en', prefix: '', expected: [] });
document.querySelector('.editor').addEventListener('input', () => { if (controller) editedDuringRun = true; });
$('export').onclick = () => {
  if (!session) return;
  const url = URL.createObjectURL(new Blob([JSON.stringify(session, null, 2) + '\n'], { type: 'application/json' }));
  const link = node('a'); link.href = url; link.download = `badi-prediction-${new Date().toISOString().replaceAll(':', '-')}.json`; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
};
$('import-button').onclick = () => $('import').click();
$('import').onchange = async () => {
  const file = $('import').files[0]; if (!file) return;
  const revision = ++importRevision;
  if (resetting) { $('import').value = ''; return; }
  try {
    if (file.size > 512 * 1024) throw new Error('Import files up to 512 KiB.');
    const data = JSON.parse(await file.text());
    // A completed file read cannot restore data after a clear or newer import.
    if (revision !== importRevision) return;
    const suite = data.schema === 'badi.prediction-lab.session.v1' ? data.suite : data;
    if (suite?.schema !== 'badi.prediction-suite.v1' || !Array.isArray(suite.cases) || !suite.cases.length || suite.cases.length > 300) throw new Error('Choose a Badi test set or exported session (up to 300 cases).');
    // Full authoritative validation occurs before any inference on the server.
    if (suite.cases.some(item => !item || typeof item.id !== 'string' || typeof item.prefix !== 'string'
      || (item.expected !== undefined && (!Array.isArray(item.expected) || item.expected.some(value => typeof value !== 'string'))))) throw new Error('The imported cases have an invalid shape.');
    cases = structuredClone(suite.cases.map(item => ({ ...item, expected: item.expected ?? [] })));
    renderCases(); fillEditor(cases[0]); setStatus(`Imported ${cases.length} cases. Previous outputs are not reused as new results.`);
  } catch (error) { if (revision === importRevision) setStatus(error.message); }
  finally { if (revision === importRevision) $('import').value = ''; }
};
$('clear-all').onclick = async () => {
  importRevision++; $('import').value = '';
  resetting = true; session = null; controller?.abort(); $('clear-all').disabled = true; $('run').disabled = true;
  qualificationUI?.clear();
  try {
    const response = await fetch('/api/reset', { method: 'POST', headers, body: '{}' });
    if (!response.ok) throw new Error('Could not reset the model.');
    cases = []; session = null; renderCases(); $('results').replaceChildren(); $('summary').replaceChildren();
    fillEditor({ id: 'new-case', language: 'en', prefix: '', expected: [] }); $('export').disabled = true;
    setStatus('Session cleared. The owned model process and its cached context are stopped.');
  } catch (error) { setStatus(error.message); } finally { resetting = false; $('clear-all').disabled = false; $('run').disabled = Boolean(controller); }
};

const modelRequest = async (path, body, signal) => {
  const response = await fetch(path, { method: body === undefined ? 'GET' : 'POST', headers, ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal });
  const value = await response.json(); if (!response.ok) throw new Error(value.message ?? value.error ?? 'Model operation failed.'); return value;
};
const qualificationUI = $('qualification') ? mountQualification({ root: $('qualification'), request: modelRequest, getSession: () => session, onStatus: setStatus }) : null;
const discoveryRoot = $('discovery');
const discoveryUI = discoveryRoot ? mountDiscovery({ root: discoveryRoot,
  request: async (path, body, signal) => {
    const response = await fetch(path, { method: body === undefined ? 'GET' : 'POST', headers, ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal });
    const value = await response.json(); if (!response.ok) throw new Error(value.message ?? value.error ?? 'Model operation failed.'); return value;
  },
  onSelected: selected => {
    qualificationUI?.clear();
    const profile = selected.details?.prompt_profile;
    for (const input of document.querySelectorAll('input[name="mode"]')) {
      input.checked = profile === 'model_template_available' ? input.value === 'native_instructed' : input.value === 'healed';
    }
    setStatus(`Selected ${selected.candidate.file} for an experiment. Qualification requires measured usefulness.`);
  },
}) : null;
$('inspect-device')?.addEventListener('click', async () => {
  const target = $('device-details'); target.textContent = 'Inspecting this laptop…';
  try {
    const response = await fetch('/api/device', { headers }); const value = await response.json();
    if (!response.ok) throw new Error(value.error); target.textContent = JSON.stringify(value, null, 2);
  } catch (error) { target.textContent = error.message; }
});
$('baseline-model')?.addEventListener('click', async () => {
  try {
    const response = await fetch('/api/model/baseline', { method: 'POST', headers, body: '{}' });
    if (!response.ok) throw new Error((await response.json()).error);
    for (const input of document.querySelectorAll('input[name="mode"]')) input.checked = input.value === 'production_baseline';
    discoveryUI?.setSelection(null);
    qualificationUI?.clear();
    setStatus('Installed model selected for the baseline comparison. Its desktop service is unchanged.');
  } catch (error) { setStatus(error.message); }
});
