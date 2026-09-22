import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { validateRunCorpus, aggregate, sha256 } from './lib.mjs';

const [beforeDirectory, afterDirectory, beforeReviewPath, afterReviewPath, outputPath, ...extra] = process.argv.slice(2);
if (!beforeDirectory || !afterDirectory || !beforeReviewPath || !afterReviewPath || !outputPath || extra.length) {
  throw new Error('Usage: node evaluation/writing/compare.mjs BEFORE_RUN AFTER_RUN BEFORE_REVIEW AFTER_REVIEW OUTPUT_JSON');
}
async function load(directory, reviewPath) {
  const read = async name => JSON.parse(await readFile(resolve(directory, name), 'utf8'));
  const run = await read('run.json');
  const summary = await read('summary.json');
  const samples = await read('samples.json');
  const reviewBytes = await readFile(resolve(reviewPath));
  if (!summary.complete || !summary.valid_run_identity) throw new Error('Comparison needs complete valid runs');
  await validateRunCorpus(run, samples);
  const metrics = aggregate(samples, JSON.parse(reviewBytes));
  if (metrics.overall.quality_review.reviewed !== metrics.overall.suggestions) throw new Error('Review every shown suggestion before comparison');
  return { run, samples, metrics, review_sha256: sha256(reviewBytes) };
}
const before = await load(beforeDirectory, beforeReviewPath);
const after = await load(afterDirectory, afterReviewPath);
if (before.run.corpus.split !== after.run.corpus.split ||
    JSON.stringify(before.samples.map(row => [row.id, row.prefix, row.language, row.reference])) !==
    JSON.stringify(after.samples.map(row => [row.id, row.prefix, row.language, row.reference]))) {
  throw new Error('Comparison requires identical ordered inputs and language metadata');
}
const difference = (a, b) => a === null || b === null ? null : b - a;
function pair(a, b) {
  return { before: a, after: b, delta: { suggestions: b.suggestions - a.suggestions, errors: b.errors - a.errors,
    useful_next_word_yield_per_request: difference(a.quality_review.useful_next_word_yield_per_request, b.quality_review.useful_next_word_yield_per_request),
    useful_words_per_reviewed_suggestion: difference(a.quality_review.useful_words_per_reviewed_suggestion, b.quality_review.useful_words_per_reviewed_suggestion),
    suggestion_p50_ms: difference(a.latency_ms.suggestions.p50, b.latency_ms.suggestions.p50),
    suggestion_p95_ms: difference(a.latency_ms.suggestions.p95, b.latency_ms.suggestions.p95) } };
}
const result = { schema: 'badi.writing-comparison.v1', split: before.run.corpus.split,
  corpus_sha256: before.run.corpus.manifest.files[before.run.corpus.split].sha256,
  identities: { before_broker: before.run.provenance.broker.executable.sha256, after_broker: after.run.provenance.broker.executable.sha256,
    before_review: before.review_sha256, after_review: after.review_sha256 },
  overall: pair(before.metrics.overall, after.metrics.overall),
  by_language: Object.fromEntries(Object.keys(before.metrics.by_language)
    .map(language => [language, pair(before.metrics.by_language[language], after.metrics.by_language[language])])),
  limitation: 'Paired synthetic inputs and attributed rubric reviews. Potential usefulness and request latency do not establish actual typing benefit, physical application support or Cotypist parity.' };
await writeFile(resolve(outputPath), JSON.stringify(result, null, 2) + '\n', { mode: 0o600, flag: 'wx' });
console.log(JSON.stringify({ output: resolve(outputPath), overall_delta: result.overall.delta }, null, 2));
