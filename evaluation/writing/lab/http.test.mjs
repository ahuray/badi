import assert from 'node:assert/strict';
import { Readable } from 'node:stream';
import { test } from 'node:test';
import { CONTENT_SECURITY_POLICY, fromLabOrigin, fromLabPage, hasLabToken, readJSON, sendJSON, setSecurityHeaders } from './http.mjs';

const origin = 'http://127.0.0.1:36889';
const capability = 'a'.repeat(64);
const request = headers => ({ headers: { host: '127.0.0.1:36889', ...headers } });

test('only the served page passes: its local host, its origin when sent, and its token', () => {
  assert.equal(fromLabOrigin(request({}), origin), true);
  assert.equal(fromLabOrigin(request({ origin }), origin), true);
  assert.equal(fromLabOrigin(request({ host: 'localhost:36889' }), origin), false);
  assert.equal(fromLabOrigin(request({ origin: 'https://foreign.example' }), origin), false);
  assert.equal(hasLabToken(request({ 'x-badi-lab-token': capability }), capability), true);
  for (const token of [undefined, '', 'b'.repeat(64), capability.slice(1), `${capability}a`]) {
    assert.equal(hasLabToken(request({ 'x-badi-lab-token': token }), capability), false, String(token));
  }
  assert.equal(fromLabPage(request({ origin, 'x-badi-lab-token': capability }), origin, capability), true);
  assert.equal(fromLabPage(request({ origin: 'null', 'x-badi-lab-token': capability }), origin, capability), false);
});

test('a body is bounded, strict UTF-8 JSON, and each route names its own errors', async () => {
  const errors = { tooLarge: () => new Error('too large'), malformed: () => new Error('malformed') };
  const body = (...chunks) => Readable.from(chunks.map(chunk => Buffer.from(chunk)));
  assert.deepEqual(await readJSON(body('{"a":', '"ü"}'), 64, errors), { a: 'ü' });
  await assert.rejects(readJSON(body('{"a":"', 'x'.repeat(64), '"}'), 64, errors), /too large/u);
  await assert.rejects(readJSON(body('{bad'), 64, errors), /malformed/u);
  await assert.rejects(readJSON(Readable.from([Buffer.from([0x22, 0xff, 0x22])]), 64, errors), /malformed/u);
});

test('responses carry the shared headers, and a reply after the client left is dropped', () => {
  const headers = {};
  setSecurityHeaders({ setHeader: (name, value) => { headers[name] = value; } });
  assert.deepEqual(headers, { 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff',
    'Cross-Origin-Resource-Policy': 'same-origin', 'Referrer-Policy': 'no-referrer', 'Content-Security-Policy': CONTENT_SECURITY_POLICY });
  const written = [];
  const response = { destroyed: false, writableEnded: false, writeHead: (...args) => written.push(args), end: value => written.push(value) };
  sendJSON(response, 409, { error: 'busy' });
  assert.deepEqual(written, [[409, { 'Content-Type': 'application/json; charset=utf-8' }], '{"error":"busy"}']);
  sendJSON({ ...response, destroyed: true }, 200, {});
  assert.equal(written.length, 2);
});
