const el = (tag, text) => { const node = document.createElement(tag); if (text !== undefined) node.textContent = text; return node; };

export function reviewsForConfiguration(session, configuration) {
  const targets = new Map(configuration.review_targets.map(target => [target.case_id, target]));
  const reviews = (session?.records ?? []).filter(record => record.config.id === configuration.config.id && record.review).map(record => {
    const target = targets.get(record.case.id);
    if (!target) throw new Error('Review targets changed. Refresh the completed run.');
    if (!record.review_target || ['run_id', 'case_id', 'config_id', 'result_sha256', 'outcome'].some(key => record.review_target[key] !== target[key])) throw new Error('This judgment belongs to a different result. Review the current addition again.');
    const judgment = { Useful: 'useful', 'Acceptable alternative': 'neutral', Unhelpful: 'unhelpful', Harmful: 'harmful' }[record.review.judgment];
    if (!judgment) throw new Error('Choose a current full-addition review judgment.');
    return { ...target, judgment, full_addition_reviewed: true, substantive: judgment === 'useful' };
  });
  for (const missing of session?.missingReviews ?? []) if (missing.config_id === configuration.config.id) {
    const target = targets.get(missing.case_id);
    if (!target || target.outcome !== 'missing' || target.run_id !== missing.run_id || target.result_sha256 !== missing.result_sha256) throw new Error('Missing-outcome review targets changed.');
    reviews.push({ ...target, judgment: 'unhelpful', full_addition_reviewed: true, substantive: false });
  }
  return reviews;
}

export function mountQualification({ root, request, getSession, onStatus = () => {} }) {
  const summary = el('p', 'Run a discovered model, review its full additions, then assess the evidence. No model is qualified yet.'); summary.setAttribute('role', 'status');
  const controls = el('div'); controls.className = 'actions';
  const select = el('select'); select.setAttribute('aria-label', 'Configuration to qualify');
  const button = label => { const value = el('button', label); value.className = 'secondary'; return value; };
  const refresh = button('Refresh qualification'), assess = button('Assess my reviews'), absent = button('Review all absent additions');
  const saveLabel = el('label'), save = el('input'); save.type = 'checkbox'; saveLabel.append(save, document.createTextNode('Save counts and measurements locally (no drafts)'));
  const duration = el('select'); duration.setAttribute('aria-label', 'Resource diagnostic duration');
  for (const [value, label] of [[30, '30 seconds · smoke'], [300, '5 minutes · diagnostic'], [1800, '30 minutes · sustained gate']]) {
    const option = el('option', label); option.value = String(value); duration.append(option);
  }
  const diagnose = button('Measure runtime behavior'), cancel = button('Cancel measurements'); cancel.hidden = true;
  const details = el('details'); details.append(el('summary', 'Qualification stages and measurements'));
  const trace = el('pre'); trace.className = 'trace'; details.append(trace);
  const loadDetails = el('details'); loadDetails.append(el('summary', 'Recheck a saved evidence receipt'));
  const receipt = el('input'); receipt.setAttribute('aria-label', 'Saved evidence ID'); receipt.maxLength = 64;
  const load = button('Recheck saved evidence'); loadDetails.append(receipt, load);
  const note = el('p', 'Useful means a substantive, correct addition with the entire displayed tail reviewed. Generic words and reference matches receive no automatic credit. Failed or absent results remain in the denominator. Diagnostic timing does not replace the 550 ms typing target.'); note.className = 'hint';
  controls.append(select, refresh, assess, absent);
  root.append(el('h3', 'Measured qualification'), summary, note, controls, saveLabel, duration, diagnose, cancel, details, loadDetails);
  let status = null, pending = null, revision = 0;
  const buttons = [refresh, assess, absent, diagnose, load];
  const configuration = () => status?.run?.configurations.find(value => value.config.id === select.value);
  const render = value => {
    status = value;
    const previous = select.value; select.replaceChildren();
    for (const config of value.run?.configurations ?? []) { const option = el('option', config.config.id); option.value = config.config.id; select.append(option); }
    if ((value.run?.configurations ?? []).some(value => value.config.id === previous)) select.value = previous;
    const recommendation = value.recommendation;
    summary.textContent = recommendation?.status === 'recommended'
      ? `Qualified recommendation: ${recommendation.candidate_id}. Select the matching model in discovery to use it in this Lab.`
      : `No qualified model. ${(recommendation?.reasons ?? ['Review and measure a discovered model to test the hard gates.']).join(' ')}`;
    trace.textContent = JSON.stringify(value, null, 2);
  };
  const operation = async callback => {
    if (pending) return;
    const current = ++revision; pending = new AbortController();
    for (const button of buttons) button.disabled = true; cancel.hidden = false;
    try { await callback(pending.signal, value => { if (current === revision) render(value); }); }
    catch (error) { if (current === revision) {
      summary.textContent = error.name === 'AbortError' ? 'Request cancelled. Refresh qualification to inspect runtime cleanup.' : error.message;
      onStatus(summary.textContent);
    } }
    finally { pending = null; cancel.hidden = true; for (const button of buttons) button.disabled = false; }
  };
  const refreshStatus = () => operation(async (signal, update) => update(await request('/api/qualification/status', undefined, signal)));
  const bound = () => {
    const session = getSession(), selected = configuration();
    if (!selected || !status.run?.finished || session?.plan?.run_id !== status.run.run_id) throw new Error('Complete and refresh the current discovered-model comparison first.');
    return { session, selected, run_id: status.run.run_id, config_id: selected.config.id };
  };
  refresh.onclick = refreshStatus;
  assess.onclick = () => operation(async (signal, update) => {
    const { session, selected, run_id, config_id } = bound();
    const result = await request('/api/qualification/review', { run_id, config_id, reviews: reviewsForConfiguration(session, selected), save: save.checked }, signal);
    if (result.saved_evidence_id) receipt.value = result.saved_evidence_id;
    update(await request('/api/qualification/status', undefined, signal));
  });
  absent.onclick = () => {
    try { const { session, selected, config_id } = bound(); let count = 0;
      for (const record of session.records) if (record.config.id === config_id && record.result.outcome !== 'suggestion') { record.review = { judgment: 'Unhelpful', reviewer: 'user', at: new Date().toISOString() }; count++; }
      session.missingReviews = [...(session.missingReviews ?? []).filter(target => target.config_id !== config_id), ...selected.review_targets.filter(target => target.outcome === 'missing')];
      count += selected.review_targets.filter(target => target.outcome === 'missing').length;
      summary.textContent = `Reviewed ${count} absent additions. Submit these with Assess my reviews.`;
    } catch (error) { summary.textContent = error.message; }
  };
  diagnose.onclick = () => operation(async (signal, update) => {
    const { run_id, config_id } = bound();
    summary.textContent = 'Measuring cold start, warm reuse, paced typing, cancellation, recovery and sustained runtime pressure…'; onStatus('Resource diagnostics are running in the selected local model.');
    await request('/api/qualification/diagnose', { run_id, config_id, duration_seconds: Number(duration.value) }, signal);
    update(await request('/api/qualification/status', undefined, signal));
    onStatus('Resource diagnostics finished. Inspect qualification for measurements and rejection reasons.');
  });
  load.onclick = () => operation(async (signal, update) => { const { run_id, config_id } = bound(); await request('/api/qualification/load', { run_id, config_id, evidence_id: receipt.value }, signal); update(await request('/api/qualification/status', undefined, signal)); });
  cancel.onclick = () => pending?.abort();
  return { refresh: refreshStatus, clear() { revision++; pending?.abort(); status = null; select.replaceChildren(); trace.textContent = ''; summary.textContent = 'The model or run changed. Run and review the current model to qualify it.'; } };
}
