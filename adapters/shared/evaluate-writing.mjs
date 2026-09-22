import { BrokerClient } from './broker-client.mjs';
import { writeFile, mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const prefixes = [
  'Please find attached the', 'Thank you for taking the time to',
  'Could you please send me the', 'I will be available tomorrow',
  'Let me know if you have', 'We need to update the documentation',
  'The deployment failed because the', 'Before restarting the service, check the',
  'The next step is to', 'This function returns a list of',
  'During the meeting we discussed', 'I would like to schedule a',
  'The main reason for this change', 'Remember to save your work before',
  'Today I learned how to', 'The results suggest that',
  'Please review the changes and', 'When the network connection is lost',
  'The application should provide', 'It would be helpful to',
];
const status = async () => JSON.parse((await promisify(execFile)(resolve(import.meta.dirname, '../../target/release/badictl'), ['status'])).stdout);
const initial = await status();
if (initial.provider !== 'local_model') throw new Error('This smoke requires the actual local model');
const client = new BrokerClient('obsidian');
const samples = [];
try {
  if (!await client.connect()) throw new Error('Allow the Obsidian adapter before running this explicit evaluation');
  for (const prefix of prefixes) {
    const started = performance.now();
    try {
      const text = await client.suggest(prefix);
      samples.push({ prefix, text, latency_ms: Math.round(performance.now() - started),
        words: text ? [...new Intl.Segmenter('en', { granularity: 'word' }).segment(text)].filter(part => part.isWordLike).length : 0 });
    } catch (error) { samples.push({ prefix, text: null, error: String(error), latency_ms: Math.round(performance.now() - started) }); }
    client.cancel();
  }
} finally { client.close(); }
const latency = samples.map(sample => sample.latency_ms).sort((a, b) => a - b);
const final = await status();
const metricsDelta = Object.fromEntries(Object.entries(final.metrics).map(([key, count]) => [key, count - initial.metrics[key]]));
const report = { provider: final.provider, metrics_delta: metricsDelta, schema: 'badi.writing-smoke.v1', at: new Date().toISOString(),
  note: 'Synthetic English smoke prefixes; manually inspect relevance. This is not a held-out quality benchmark.',
  requests: samples.length, suggestions: samples.filter(sample => sample.text).length,
  errors: samples.filter(sample => sample.error).length,
  latency_ms: { p50: latency[Math.ceil(latency.length * .5) - 1], p95: latency[Math.ceil(latency.length * .95) - 1] }, samples };
const directory = resolve(import.meta.dirname, '../../output/writing');
await mkdir(directory, { recursive: true });
await writeFile(resolve(directory, 'smoke.json'), JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify(report, null, 2));
