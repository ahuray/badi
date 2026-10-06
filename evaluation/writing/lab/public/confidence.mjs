const judgments = new Set(['Useful', 'Acceptable alternative', 'Unhelpful', 'Harmful']);

function summarizeGroup(label, records) {
  const onTime = records.filter(record => record.result?.outcome === 'suggestion'
    && record.result.within_production_budget === true);
  const scored = onTime.filter(record => Number.isFinite(record.result.candidate_mean_token_logprob)
    && Number.isInteger(record.result.candidate_logprob_token_count)
    && record.result.candidate_logprob_token_count > 0);
  const scores = [...new Set(scored.map(record => record.result.candidate_mean_token_logprob))].sort((a, b) => b - a);
  const points = scores.map(threshold => {
    const retained = scored.filter(record => record.result.candidate_mean_token_logprob >= threshold);
    const reviewed = retained.filter(record => judgments.has(record.review?.judgment));
    const useful = reviewed.filter(record => record.review.judgment === 'Useful').length;
    const harmful = reviewed.filter(record => record.review.judgment === 'Harmful').length;
    const complete = reviewed.length === retained.length;
    return { threshold, retained: retained.length, reviewed: reviewed.length, useful, harmful,
      coverage: retained.length / records.length,
      useful_per_request: complete ? useful / records.length : null,
      precision: complete ? useful / retained.length : null };
  });
  return { label, requests: records.length, on_time_suggestions: onTime.length,
    scored_suggestions: scored.length, missing_scores: onTime.length - scored.length,
    reviewed_scores: scored.filter(record => judgments.has(record.review?.judgment)).length,
    points };
}

export function confidenceCoverage(records) {
  const selected = records.filter(record => record.config?.mode === 'context_confidence');
  if (!selected.length) return [];
  const languages = [...new Set(selected.map(record => record.case?.language).filter(Boolean))].sort();
  return [summarizeGroup('All languages', selected),
    ...languages.map(language => summarizeGroup(language, selected.filter(record => record.case.language === language)))];
}
