import test from 'node:test';
import assert from 'node:assert/strict';
import { parseGgufMetadata } from './gguf.mjs';
const u32 = n => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const u64 = n => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };
const str = text => Buffer.concat([u64(Buffer.byteLength(text)), Buffer.from(text)]);
const file = fields => Buffer.concat([Buffer.from('GGUF'), u32(3), u64(0), u64(fields.length), ...fields.map(([key, value]) => Buffer.concat([str(key), u32(8), str(value)]))]);
test('GGUF binds actual tokenizer bytes separately from architecture metadata', () => {
  const a = parseGgufMetadata(file([['general.architecture', 'llama'], ['tokenizer.ggml.model', 'gpt2']]));
  const b = parseGgufMetadata(file([['general.architecture', 'granite'], ['tokenizer.ggml.model', 'gpt2']]));
  const c = parseGgufMetadata(file([['general.architecture', 'llama'], ['tokenizer.ggml.model', 'llama']]));
  assert.equal(a.architecture, 'llama'); assert.equal(a.tokenizer_sha256, b.tokenizer_sha256); assert.notEqual(a.tokenizer_sha256, c.tokenizer_sha256);
});
test('GGUF rejects truncation, duplicate keys, unsafe counts, nested arrays and missing architecture', () => {
  const valid = file([['general.architecture', 'llama']]);
  for (let i = 0; i < valid.length; i++) assert.throws(() => parseGgufMetadata(valid.subarray(0, i)));
  assert.throws(() => parseGgufMetadata(file([['general.architecture', 'a'], ['general.architecture', 'b']])));
  assert.throws(() => parseGgufMetadata(file([])));
  assert.throws(() => parseGgufMetadata(Buffer.concat([Buffer.from('GGUF'), u32(3), u64(0), u64(9000)])));
  assert.throws(() => parseGgufMetadata(Buffer.concat([Buffer.from('GGUF'), u32(3), u64(0), u64(1), str('bad'), u32(9), u32(9), u64(1)])));
});
