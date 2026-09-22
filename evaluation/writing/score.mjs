import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { aggregate, sha256, validateRunCorpus } from './lib.mjs';

const [directory, reviewPath, outputPath, ...extra] = process.argv.slice(2);
if (!directory || !reviewPath || !outputPath || extra.length) {
  throw new Error('Usage: node evaluation/writing/score.mjs RUN_DIRECTORY REVIEW_JSON OUTPUT_JSON');
}
const sampleBytes = await readFile(resolve(directory, 'samples.json'));
const reviewBytes = await readFile(resolve(reviewPath));
const summary = JSON.parse(await readFile(resolve(directory, 'summary.json'), 'utf8'));
if (!summary.complete || !summary.valid_run_identity) throw new Error('Cannot score an incomplete or invalid run');
const samples = JSON.parse(sampleBytes);
const run = JSON.parse(await readFile(resolve(directory, 'run.json'), 'utf8'));
await validateRunCorpus(run, samples);
if (summary.split !== run.corpus.split) throw new Error('Summary split differs from recorded run');
const review = JSON.parse(reviewBytes);
const score = { schema: 'badi.writing-reviewed-summary.v1', split: summary.split,
  sample_sha256: sha256(sampleBytes), review_sha256: sha256(reviewBytes),
  reviewer: { kind: review.reviewer_kind, name: review.reviewer, independent_of_implementation: review.independent_of_implementation },
  review_limits: review.review_limits ?? 'Reviewer did not provide additional limitations.',
  measurements: aggregate(samples, review),
  limitation: 'Synthetic corpus and rubric review; not observed human acceptance, typing speed, adapter UI latency or Cotypist parity.' };
await writeFile(resolve(outputPath), JSON.stringify(score, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
console.log(JSON.stringify({ output: resolve(outputPath), reviewer: score.reviewer, review_limits: score.review_limits,
  overall: score.measurements.overall.quality_review }, null, 2));
