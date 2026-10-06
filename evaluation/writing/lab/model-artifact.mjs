import { constants } from 'node:fs';
import { open, realpath, lstat } from 'node:fs/promises';
import { isAbsolute, resolve } from 'node:path';
import { createHash } from 'node:crypto';

const MAX_DESCRIPTOR_BYTES = 16 * 1024;
const KEYS = ['alias', 'bytes', 'schema', 'sha256', 'weights_path'];
const fail = () => { throw new Error('Invalid Lab model artifact descriptor.'); };

export function modelArtifactPath(value) {
  if (typeof value !== 'string' || !value.isWellFormed() || !isAbsolute(value) || resolve(value) !== value
    || Buffer.byteLength(value) > 4096 || /[\p{Cc}]/u.test(value)) fail();
  return value;
}

export function validateModelArtifact(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || JSON.stringify(Object.keys(value).sort()) !== JSON.stringify(KEYS)
    || value.schema !== 'badi.lab-model-artifact.v1'
    || typeof value.sha256 !== 'string' || !/^[a-f0-9]{64}$/u.test(value.sha256)
    || !Number.isSafeInteger(value.bytes) || value.bytes < 1 || value.bytes > 8589934592
    || typeof value.alias !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,95}$/u.test(value.alias)) fail();
  modelArtifactPath(value.weights_path);
  return value;
}

function parseDescriptor(bytes) {
  if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) fail();
  let text, value;
  try { text = new TextDecoder('utf-8', { fatal: true }).decode(bytes); value = JSON.parse(text); }
  catch { fail(); }
  validateModelArtifact(value);
  // This descriptor is a flat object of strings and one unsigned integer.
  // Preserve Rust's duplicate-field and integer-token rejection at this boundary.
  const body = text.trim().slice(1, -1);
  const pair = /\s*("(?:[^"\\]|\\.)*")\s*:\s*("(?:[^"\\]|\\.)*"|(?:0|[1-9][0-9]*))\s*(,|$)/gy;
  const seen = new Set(); let end = 0;
  for (;;) {
    const match = pair.exec(body);
    if (!match || seen.has(JSON.parse(match[1]))) fail();
    seen.add(JSON.parse(match[1])); end = pair.lastIndex;
    if (!match[3]) break;
  }
  if (end !== body.length || seen.size !== KEYS.length) fail();
  return value;
}

export async function readModelArtifact(path) {
  modelArtifactPath(path);
  if (await realpath(path) !== path) fail();
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const before = await handle.stat({ bigint: true });
    if (!before.isFile() || before.size > BigInt(MAX_DESCRIPTOR_BYTES)) fail();
    const buffer = Buffer.alloc(MAX_DESCRIPTOR_BYTES + 1);
    let length = 0;
    while (length < buffer.length) {
      const { bytesRead } = await handle.read(buffer, length, buffer.length - length, length);
      if (!bytesRead) break;
      length += bytesRead;
    }
    const after = await handle.stat({ bigint: true });
    if (length > MAX_DESCRIPTOR_BYTES || ['dev', 'ino', 'size', 'mtimeNs', 'ctimeNs'].some(key => before[key] !== after[key])) fail();
    const source = buffer.subarray(0, length);
    const descriptor = parseDescriptor(source);
    if (await realpath(descriptor.weights_path) !== descriptor.weights_path
      || !(await lstat(descriptor.weights_path)).isFile()) fail();
    // Only Rust verifies weight bytes and the runtime. This is the frozen claim.
    return { path, sha256: createHash('sha256').update(source).digest('hex'), bytes: source.length, descriptor, source };
  } finally { await handle.close(); }
}

export const modelArtifactProvenance = artifact => artifact ? {
  path: artifact.path, sha256: artifact.sha256, bytes: artifact.bytes, descriptor: artifact.descriptor,
} : null;

export const modelArtifactArgs = artifact => artifact ? ['--model-artifact', artifact.path] : [];

export async function assertModelArtifactUnchanged(artifact) {
  if (artifact && (await readModelArtifact(artifact.path)).sha256 !== artifact.sha256) {
    throw new Error('The frozen Lab model artifact descriptor changed.');
  }
}

export function assertModelArtifactIdentity(identity, artifact) {
  if (artifact && (!identity || identity.model_origin !== 'explicit_lab_artifact'
    || identity.model_sha256 !== artifact.descriptor.sha256 || identity.model_size !== artifact.descriptor.bytes
    || identity.model_alias !== artifact.descriptor.alias)) {
    throw new Error('The runtime model identity differs from the frozen Lab artifact.');
  }
}
