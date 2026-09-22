import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, writeFile, rm, symlink, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import { modelArtifactPath, validateModelArtifact, readModelArtifact, modelArtifactArgs,
  assertModelArtifactUnchanged, assertModelArtifactIdentity } from './model-artifact.mjs';

const value = weights_path => ({ schema: 'badi.lab-model-artifact.v1', weights_path,
  sha256: 'a'.repeat(64), bytes: 1107409024, alias: 'fixture-artifact' });

test('artifact schema permits only bounded explicit identities and canonical absolute paths', () => {
  const valid = value('/fixture/model.gguf');
  assert.equal(validateModelArtifact(valid), valid);
  for (const change of [v => v.extra = true, v => v.schema = 'other', v => v.sha256 = 'A'.repeat(64),
    v => v.bytes = 0, v => v.bytes = 8589934593, v => v.bytes = 1.5, v => v.bytes = '1',
    v => v.alias = '-wrong', v => v.alias = 'نام', v => v.alias = 'a'.repeat(97),
    v => v.weights_path = 'relative.gguf', v => v.weights_path = '/tmp/../weights.gguf',
    v => v.weights_path += '\n', v => v.weights_path = '/' + 'é'.repeat(2048)]) {
    const changed = structuredClone(valid); change(changed); assert.throws(() => validateModelArtifact(changed));
  }
  assert.throws(() => modelArtifactPath('/tmp//descriptor.json'));
  assert.deepEqual(modelArtifactArgs(null), []);
  assert.doesNotThrow(() => assertModelArtifactIdentity({ existing: 'default' }, null));
});

test('bounded descriptor reads preserve bytes and reject duplicates, symlink escapes and changed claims', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'badi-artifact-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const weights = join(directory, 'fixture.gguf'), path = join(directory, 'descriptor.json');
  await writeFile(weights, 'fixture');
  const descriptor = value(weights), source = JSON.stringify(descriptor, null, 2) + '\n';
  await writeFile(path, source);
  const artifact = await readModelArtifact(path);
  assert.equal(artifact.source.toString(), source);
  assert.equal(artifact.sha256, createHash('sha256').update(source).digest('hex'));
  assert.deepEqual(modelArtifactArgs(artifact), ['--model-artifact', path]);
  await assertModelArtifactUnchanged(artifact);
  // Node freezes the descriptor claim; the tiny fixture intentionally does not
  // match its advertised weight bytes. Rust remains the verifier of the model.
  assert.equal(artifact.descriptor.bytes, 1107409024);
  const redirect = join(directory, 'redirect.json'); await symlink(path, redirect);
  await assert.rejects(readModelArtifact(redirect));
  const directoryLink = join(directory, 'link'); await symlink(directory, directoryLink);
  await assert.rejects(readModelArtifact(join(directoryLink, 'descriptor.json')));
  const weightLink = join(directory, 'weight-link'); await symlink(weights, weightLink);
  await mkdir(join(directory, 'not-weights'));
  const invalid = [JSON.stringify({ ...descriptor, weights_path: weightLink }),
    JSON.stringify({ ...descriptor, weights_path: join(directory, 'not-weights') }),
    source.replace('"bytes": 1107409024', '"bytes": 1107409024.0'),
    source.replace('"bytes": 1107409024', '"bytes": 1e3'),
    source.replace('{', '{"alias":"duplicate",'),
    source.replace('{', '{"\\u0061lias":"escaped-duplicate",'),
    '{PRIVATE_INVALID_JSON', Buffer.from([0xff]), '\ufeff' + source, ' '.repeat(16385)];
  for (const bytes of invalid) { await writeFile(path, bytes); await assert.rejects(readModelArtifact(path)); }
  await writeFile(path, source + ' ');
  await assert.rejects(assertModelArtifactUnchanged(artifact), /changed/u);
});

test('every explicit ready identity field must match, and none is inferred from a fixture', () => {
  const descriptor = value('/fixture/model.gguf');
  const artifact = { descriptor };
  const identity = { model_origin: 'explicit_lab_artifact', model_sha256: descriptor.sha256,
    model_size: descriptor.bytes, model_alias: descriptor.alias };
  assert.doesNotThrow(() => assertModelArtifactIdentity(identity, artifact));
  for (const key of Object.keys(identity)) {
    const changed = { ...identity }; delete changed[key];
    assert.throws(() => assertModelArtifactIdentity(changed, artifact));
  }
  for (const changed of [{ ...identity, model_origin: 'catalog' }, { ...identity, model_size: 7 },
    { ...identity, model_sha256: 'b'.repeat(64) }, { ...identity, model_alias: 'other' }]) {
    assert.throws(() => assertModelArtifactIdentity(changed, artifact));
  }
});
