// Bounded GGUF metadata inspection. Never reads tensors or executes repository code.
// Format: https://github.com/ggml-org/ggml/blob/master/docs/gguf.md
import { constants } from 'node:fs';
import { open, realpath } from 'node:fs/promises';
import { createHash } from 'node:crypto';

const MAX_HEADER = 32 * 1024 * 1024;
const MAX_ITEMS = 500000;
const fail = () => { throw new Error('Unsupported, truncated or oversized GGUF metadata.'); };

export function parseGgufMetadata(buffer) {
  let offset = 0;
  const bytes = count => {
    if (!Number.isSafeInteger(count) || count < 0 || offset + count > buffer.length || offset + count > MAX_HEADER) fail();
    const value = buffer.subarray(offset, offset + count); offset += count; return value;
  };
  const u32 = () => bytes(4).readUInt32LE();
  const u64 = () => { const value = bytes(8).readBigUInt64LE(); if (value > BigInt(Number.MAX_SAFE_INTEGER)) fail(); return Number(value); };
  const string = () => new TextDecoder('utf-8', { fatal: true }).decode(bytes(u64()));
  if (bytes(4).toString() !== 'GGUF') fail();
  const version = u32(); if (![2, 3].includes(version)) fail();
  const tensors = u64(), entries = u64();
  if (tensors > MAX_ITEMS || entries > 4096) fail();
  const readValue = (type, retain = true) => {
    switch (type) {
      case 0: return bytes(1).readUInt8();
      case 1: return bytes(1).readInt8();
      case 2: return bytes(2).readUInt16LE();
      case 3: return bytes(2).readInt16LE();
      case 4: return u32();
      case 5: return bytes(4).readInt32LE();
      case 6: return bytes(4).readFloatLE();
      case 7: { const value = bytes(1)[0]; if (value > 1) fail(); return value === 1; }
      case 8: return string();
      case 9: {
        const element = u32(), count = u64();
        if (element === 9 || count > MAX_ITEMS) fail();
        const values = [];
        for (let i = 0; i < count; i++) { const value = readValue(element); if (retain && count <= 256) values.push(value); }
        return retain && count <= 256 ? values : { count, element_type: element };
      }
      case 10: return u64();
      case 11: { const n = bytes(8).readBigInt64LE(); if (n < BigInt(Number.MIN_SAFE_INTEGER) || n > BigInt(Number.MAX_SAFE_INTEGER)) fail(); return Number(n); }
      case 12: return bytes(8).readDoubleLE();
      default: return fail();
    }
  };
  const metadata = Object.create(null), tokenizer = createHash('sha256');
  let tokenizerEntries = 0;
  for (let i = 0; i < entries; i++) {
    const start = offset, key = string();
    if (key.length > 512 || Object.hasOwn(metadata, key)) fail();
    metadata[key] = readValue(u32());
    if (key.startsWith('tokenizer.')) { tokenizer.update(buffer.subarray(start, offset)); tokenizerEntries++; }
  }
  const architecture = metadata['general.architecture'];
  if (typeof architecture !== 'string' || !/^[a-z0-9_]{1,64}$/u.test(architecture)) fail();
  return { schema: 'badi.gguf-metadata.v1', version, tensors, metadata_bytes: offset,
    architecture, tokenizer_sha256: tokenizerEntries ? tokenizer.digest('hex') : null,
    tokenizer_hash_contract: 'sha256-ordered-raw-gguf-tokenizer-entries-v1', metadata };
}

export async function inspectGguf(path) {
  if (await realpath(path) !== path) fail();
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const before = await handle.stat({ bigint: true });
    if (!before.isFile()) fail();
    const buffer = Buffer.alloc(Number(before.size > BigInt(MAX_HEADER) ? BigInt(MAX_HEADER) : before.size));
    let length = 0;
    while (length < buffer.length) { const { bytesRead } = await handle.read(buffer, length, buffer.length - length, length); if (!bytesRead) break; length += bytesRead; }
    const result = parseGgufMetadata(buffer.subarray(0, length));
    const after = await handle.stat({ bigint: true });
    if (['dev', 'ino', 'size', 'mtimeNs', 'ctimeNs'].some(key => before[key] !== after[key])) fail();
    return result;
  } finally { await handle.close(); }
}
