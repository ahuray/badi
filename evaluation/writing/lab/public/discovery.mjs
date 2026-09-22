const bytes = value => Number.isFinite(value) ? `${(value / 1024 ** 2).toFixed(0)} MiB` : 'unknown size';

export function mountDiscovery({ root, request, onSelected = () => {} }) {
  const document = root.ownerDocument;
  const node = (tag, text, className) => {
    const result = document.createElement(tag); if (text !== undefined) result.textContent = text;
    if (className) result.className = className; return result;
  };
  let active = null; let generation = 0; let poll = null; let resetting = false; let resetFailed = false;
  const heading = node('h2', 'Find a small local model');
  const explanation = node('p', 'Search public Hugging Face metadata, inspect the device fit, then verify a download for a local Lab comparison. Search terms leave this laptop; your drafts and expected answers stay local.', 'hint');
  const form = node('form', undefined, 'discovery-search');
  const label = node('label', 'Model name or family');
  const search = node('input'); search.type = 'search'; search.maxLength = 120; search.placeholder = 'e.g. SmolLM2-360M GGUF'; search.required = true;
  label.append(search);
  const submit = node('button', 'Search Hugging Face'); submit.type = 'submit';
  const cancel = node('button', 'Cancel', 'secondary'); cancel.type = 'button'; cancel.hidden = true;
  form.append(label, submit, cancel);
  const status = node('p', 'No model is qualified from metadata alone.', 'discovery-status'); status.setAttribute('role', 'status'); status.setAttribute('aria-live', 'polite');
  const selected = node('p', 'Select a verified candidate below to change the model used by this Lab session.', 'hint');
  const results = node('div', undefined, 'discovery-results');
  root.classList.add('discovery'); root.replaceChildren(heading, explanation, form, status, selected, results);

  const api = async (operation, body, signal) => request(`/api/discovery/${operation}`, body, signal);
  const setSelection = value => {
    selected.textContent = value ? `Selected for experiment: ${value.candidate.repo} · ${value.candidate.quantization ?? 'GGUF'}. No prediction-quality qualification yet.`
      : 'The existing Lab baseline is selected.';
  };
  const details = value => { const element = node('details'); element.append(node('summary', 'Inspect metadata and measurements'), node('pre', JSON.stringify(value, null, 2), 'trace')); return element; };
  const controls = busy => { for (const button of root.querySelectorAll('button')) if (button !== cancel) button.disabled = busy || button.dataset.blocked === 'true'; cancel.hidden = !busy; };
  async function run(action) {
    if (active || resetting || resetFailed) return;
    const controller = new AbortController(); active = controller; const epoch = generation; controls(true);
    try { await action(controller.signal, () => epoch === generation && !controller.signal.aborted && active === controller); }
    catch (error) { if (epoch === generation) status.textContent = controller.signal.aborted ? 'Cancelled. A partial download can resume.' : error.message; }
    finally { clearTimeout(poll); if (active === controller) active = null; controls(Boolean(active || resetting || resetFailed)); }
  }
  function button(text, action, blocked = false) {
    const value = node('button', text, 'secondary'); value.type = 'button'; value.dataset.blocked = String(blocked); value.disabled = blocked;
    value.onclick = () => run(action); return value;
  }
  function showCandidates(result) {
    results.replaceChildren(); status.textContent = result.warning ?? `Pinned ${result.model.repo} at ${result.model.revision.slice(0, 12)}. ${result.cache_state} metadata.`;
    if (!result.candidates.length) results.append(details(result.model));
    for (const candidate of result.candidates) {
      const article = node('article', undefined, 'discovery-candidate');
      article.append(node('h3', candidate.file), node('p', `${bytes(candidate.bytes)} · ${candidate.quantization ?? 'quantization unknown'} · ${candidate.architecture ?? 'architecture unknown'} · ${(candidate.languages ?? []).join(', ') || 'languages unknown'} · ${candidate.license ?? 'license unknown'}`));
      article.append(node('p', candidate.provenance.certainty, 'hint'));
      if (candidate.rejection_reasons.length) article.append(node('p', candidate.rejection_reasons.join(' '), 'discovery-rejection'));
      const assessmentNode = node('div'); article.append(assessmentNode);
      article.append(button('Assess this device', async (signal, current) => {
        status.textContent = 'Checking compatibility and current resource headroom…';
        const assessed = await api('assess', { candidate_id: candidate.candidate_id }, signal); if (!current()) return;
        assessmentNode.replaceChildren(details(assessed.assessment));
        status.textContent = assessed.download_allowed ? 'Estimated to fit. Loading, speed and prediction quality remain unverified.' : 'This candidate failed the current fit or compatibility gates. Inspect the reasons below.';
        assessmentNode.append(node('p', (assessed.assessment.rejection_reasons ?? []).join(' '), 'discovery-rejection'));
        assessmentNode.append(button('Download and verify', async (downloadSignal, stillCurrent) => {
          status.textContent = 'Verifying the pinned artifact; completed downloads are reused.';
          const progress = async () => {
            try { const value = await api('status', undefined, downloadSignal); if (stillCurrent() && value.progress?.state === 'downloading') status.textContent = `Downloading ${bytes(value.progress.received_bytes)} / ${bytes(value.progress.total_bytes)}. Cancel preserves partial bytes.`; }
            catch { /* The main operation reports errors; polling is informational. */ }
            if (stillCurrent()) poll = setTimeout(progress, 1000);
          };
          poll = setTimeout(progress, 1000);
          const downloaded = await api('download', { candidate_id: candidate.candidate_id }, downloadSignal); if (!stillCurrent()) return;
          clearTimeout(poll);
          status.textContent = `${downloaded.reused ? 'Reused' : 'Downloaded'} and verified exact bytes. Select it to compare locally; it is not yet qualified.`;
          assessmentNode.append(button('Select for Lab comparison', async (selectionSignal, selectionCurrent) => {
            const value = await api('select', { artifact_id: downloaded.artifact_id }, selectionSignal); if (!selectionCurrent()) return;
            setSelection(value.selected);
            status.textContent = 'Model selected. Run your prediction test set and inspect complete displayed additions and timings.';
            await onSelected(value.selected);
          }));
        }, !assessed.download_allowed));
      }));
      article.append(details(candidate)); results.append(article);
    }
  }
  async function showSearch(query, cursor, signal, current) {
    status.textContent = 'Searching public Hub metadata…';
    const value = await api('search', { query, ...(cursor ? { cursor } : {}) }, signal); if (!current()) return;
    if (!cursor) results.replaceChildren();
    status.textContent = value.warning ?? `${value.models.length} results on page ${value.page} of at most ${value.page_limit}. ${value.cache_state} metadata; discovery is not qualification.`;
    for (const model of value.models) {
      const article = node('article', undefined, 'discovery-candidate');
      const link = node('a', model.repo); link.href = model.url; link.target = '_blank'; link.rel = 'noopener noreferrer';
      const title = node('h3'); title.append(link); article.append(title);
      article.append(node('p', `${model.task ?? 'task unknown'} · ${(model.languages ?? []).join(', ') || 'languages unknown'} · ${model.license ?? 'license unknown'}`));
      article.append(button('Inspect GGUF artifacts', async (inspectSignal, valid) => {
        status.textContent = 'Reading pinned file metadata…'; const result = await api('inspect', { model_id: model.model_id }, inspectSignal); if (valid()) showCandidates(result);
      })); results.append(article);
    }
    if (value.next_cursor) {
      const next = button('Next page', async (nextSignal, valid) => {
        await showSearch(query, value.next_cursor, nextSignal, valid);
        if (valid()) next.dataset.blocked = 'true';
      });
      results.append(next);
    }
  }
  form.onsubmit = event => { event.preventDefault(); return run((signal, current) => showSearch(search.value.trim(), null, signal, current)); };
  cancel.onclick = async () => {
    if (resetting) return;
    resetting = true; generation++; active?.abort(); clearTimeout(poll); controls(true);
    try { await api('reset', {}); resetFailed = false; status.textContent = 'Cancelled. Partial downloads can resume against the same pinned revision.'; }
    catch (error) { resetFailed = true; status.textContent = `Could not confirm cancellation: ${error.message}`; }
    finally { resetting = false; active = null; controls(resetFailed); }
  };
  return { async reset() { await cancel.onclick(); }, setSelection, root };
}
