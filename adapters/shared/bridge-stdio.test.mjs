import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { PassThrough } from 'node:stream';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  DEFAULT_IDLE_MS,
  LINE_LIMIT,
  bridgeIdleMs,
  createBridgeStdio,
  parseBridgeLine,
} from './bridge-stdio.mjs';

const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

test('bridgeIdleMs parses bounded positive millisecond overrides and defaults safely', () => {
  assert.equal(bridgeIdleMs({}), DEFAULT_IDLE_MS);
  assert.equal(bridgeIdleMs({ BADI_BRIDGE_IDLE_MS: '' }), DEFAULT_IDLE_MS);
  assert.equal(bridgeIdleMs({ BADI_BRIDGE_IDLE_MS: '0' }), DEFAULT_IDLE_MS);
  assert.equal(bridgeIdleMs({ BADI_BRIDGE_IDLE_MS: '-100' }), DEFAULT_IDLE_MS);
  assert.equal(bridgeIdleMs({ BADI_BRIDGE_IDLE_MS: '0100' }), DEFAULT_IDLE_MS);
  assert.equal(bridgeIdleMs({ BADI_BRIDGE_IDLE_MS: '250ms' }), DEFAULT_IDLE_MS);
  assert.equal(bridgeIdleMs({ BADI_BRIDGE_IDLE_MS: '1000000000' }), DEFAULT_IDLE_MS);
  assert.equal(bridgeIdleMs({ BADI_BRIDGE_IDLE_MS: '1' }), 1);
  assert.equal(bridgeIdleMs({ BADI_BRIDGE_IDLE_MS: '300' }), 300);
  assert.equal(bridgeIdleMs({ BADI_BRIDGE_IDLE_MS: '999999999' }), 999_999_999);
});

test('parseBridgeLine enforces line length, JSON syntax and plain object commands', () => {
  assert.deepEqual(parseBridgeLine('{"op":"status","id":1}'), {
    ok: true,
    command: { op: 'status', id: 1 },
  });
  const exact = `{"padding":"${'a'.repeat(LINE_LIMIT - 14)}"}`;
  assert.equal(exact.length, LINE_LIMIT);
  assert.equal(parseBridgeLine(exact).ok, true);
  assert.deepEqual(parseBridgeLine(`${exact} `), { ok: false, reason: 'input_too_large' });
  assert.deepEqual(parseBridgeLine('{bad'), { ok: false, reason: 'invalid_json' });
  assert.deepEqual(parseBridgeLine(''), { ok: false, reason: 'invalid_json' });
  assert.deepEqual(parseBridgeLine('null'), { ok: false, reason: 'invalid_json' });
  assert.deepEqual(parseBridgeLine('[]'), { ok: false, reason: 'invalid_object' });
  assert.deepEqual(parseBridgeLine('123'), { ok: false, reason: 'invalid_object' });
  assert.deepEqual(parseBridgeLine('"text"'), { ok: false, reason: 'invalid_object' });
  assert.deepEqual(parseBridgeLine('true'), { ok: false, reason: 'invalid_object' });
});

test('sequential mode processes commands in order, holds idle exit while active, and exits after quiet', async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  const written = [];
  output.on('data', chunk => written.push(chunk.toString('utf8')));
  let closedCount = 0;
  const stdio = createBridgeStdio({
    input,
    output,
    idleMs: 70,
    lineLimit: 64,
    onClose: () => { closedCount++; },
  });

  const running = stdio.serve({
    onInvalid({ reason }) {
      stdio.writeLine(`INVALID ${reason}`);
    },
    async onCommand(command) {
      if (command.slow) await delay(120);
      stdio.writeLine(`OK ${command.n}`);
    },
  });

  input.write('{"n":1,"slow":true}\n{"n":2}\n');
  input.write(`${'x'.repeat(65)}\n[]\n{bad\n`);
  await delay(140);
  assert.equal(stdio.closed, false, 'slow active command holds idle exit past idleMs');
  assert.deepEqual(written.join('').trim().split('\n'), [
    'OK 1',
    'OK 2',
    'INVALID input_too_large',
    'INVALID invalid_object',
    'INVALID invalid_json',
  ]);

  await running;
  assert.equal(stdio.closed, true);
  assert.equal(closedCount, 1);
  assert.equal(input.destroyed, true);
});

test('concurrent mode suppresses idle exit while any request runs or focus is held', async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  const replies = [];
  output.on('data', chunk => {
    for (const line of chunk.toString('utf8').split('\n').filter(Boolean)) {
      replies.push(JSON.parse(line));
    }
  });
  let closedCount = 0;
  const stdio = createBridgeStdio({
    input,
    output,
    idleMs: 70,
    concurrent: true,
    onClose: () => { closedCount++; },
  });

  const order = [];
  const running = stdio.serve({
    onInvalid() {
      stdio.writeJson({ ok: false, reason: 'invalid_request' });
    },
    async onCommand(command) {
      order.push(`start:${command.id}`);
      if (command.op === 'focus') {
        stdio.setHeld(command.focused);
        stdio.writeJson({ id: command.id, ok: true });
        return;
      }
      await delay(command.ms);
      stdio.writeJson({ id: command.id, ok: true });
    },
  });

  // A fast second command finishing early must not arm idle exit while the
  // slow first command is still running past idleMs.
  input.write('{"id":1,"ms":130}\n{"id":2,"ms":10}\n');
  await delay(95);
  assert.deepEqual(order, ['start:1', 'start:2']);
  assert.deepEqual(replies, [{ id: 2, ok: true }]);
  assert.equal(stdio.closed, false, 'in-flight command 1 still suppresses idle exit');
  await delay(50);
  assert.deepEqual(replies, [{ id: 2, ok: true }, { id: 1, ok: true }]);

  // Focusing the editor suppresses idle exit until focus is released.
  input.write('{"id":3,"op":"focus","focused":true}\n');
  await delay(130);
  assert.equal(stdio.held, true);
  assert.equal(stdio.idleArmed, false);
  assert.equal(stdio.closed, false, 'focused editor suppresses idle exit');

  input.write('{"id":4,"op":"focus","focused":false}\n');
  await delay(20);
  assert.equal(stdio.held, false);
  assert.equal(stdio.idleArmed, true);

  await running;
  assert.equal(stdio.closed, true);
  assert.equal(closedCount, 1);
});

test('EOF teardown, pre-closed input, idempotent close and broken output pipes stay clean', async () => {
  // Half-closed stdin (piped command followed by EOF) still delivers the reply on stdout.
  {
    const input = new PassThrough();
    const output = new PassThrough();
    const chunks = [];
    output.on('data', data => chunks.push(data.toString('utf8')));
    let closedCount = 0;
    const stdio = createBridgeStdio({
      input,
      output,
      concurrent: true,
      onClose: () => { closedCount++; },
    });
    const done = stdio.serve({
      async onCommand(command) {
        await delay(15);
        stdio.writeJson({ id: command.id, ok: true });
      },
    });
    input.end('{"id":7}\n');
    await done;
    await delay(25);
    stdio.close();
    assert.equal(closedCount, 1);
    assert.equal(chunks.join(''), '{"id":7,"ok":true}\n');
  }

  // Input already ended before serve() starts closes cleanly once.
  {
    const input = new PassThrough();
    const output = new PassThrough();
    let closedCount = 0;
    const stdio = createBridgeStdio({ input, output, onClose: () => { closedCount++; } });
    input.end();
    await delay(10);
    await stdio.serve({ onCommand() {} });
    assert.equal(closedCount, 1);
  }

  // Broken output pipe (EPIPE) closes the bridge without throwing or crashing.
  {
    const input = new PassThrough();
    const output = new PassThrough();
    let closedCount = 0;
    const stdio = createBridgeStdio({ input, output, onClose: () => { closedCount++; } });
    const done = stdio.serve({ onCommand() {} });
    const epipe = Object.assign(new Error('write EPIPE'), { code: 'EPIPE' });
    output.emit('error', epipe);
    await done;
    assert.equal(stdio.closed, true);
    assert.equal(closedCount, 1);
    output.destroy();
    assert.equal(stdio.writeLine('after destroy'), false);
  }
});

test('the Neovim bridge holds across idleMs while focused and exits cleanly once unfocused', async () => {
  const root = await mkdtemp('/tmp/badi-bridge-stdio-');
  const bridgePath = fileURLToPath(new URL('../neovim/bridge.mjs', import.meta.url));
  const child = spawn(process.execPath, [bridgePath], {
    env: { ...process.env, XDG_RUNTIME_DIR: join(root, 'no-broker'), BADI_BRIDGE_IDLE_MS: '120' },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  let stderr = '';
  child.stderr.on('data', chunk => { stderr += chunk; });
  const reader = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  const nextReply = async () => {
    for (;;) {
      const { value, done } = await reader.next();
      if (done) throw new Error('Bridge closed stdout before replying');
      const parsed = JSON.parse(value);
      if (parsed.event === undefined) return parsed;
    }
  };

  try {
    child.stdin.write(`${'x'.repeat(LINE_LIMIT + 1)}\n[]\nnull\n`);
    assert.deepEqual(await nextReply(), { ok: false, reason: 'invalid_request' });
    assert.deepEqual(await nextReply(), { ok: false, reason: 'invalid_request' });
    assert.deepEqual(await nextReply(), { ok: false, reason: 'invalid_request' });

    child.stdin.write('{"id":1,"op":"focus","focused":true}\n');
    assert.deepEqual(await nextReply(), { id: 1, ok: true, withheld: false });
    await delay(220);
    assert.equal(child.exitCode, null, 'terminal focus suppresses bridge idle exit');

    child.stdin.write('{"id":2,"op":"focus","focused":false}\n');
    assert.deepEqual(await nextReply(), { id: 2, ok: true, withheld: false });
    const [code] = await once(child, 'exit');
    assert.equal(code, 0, stderr);
    assert.equal(stderr, '');
  } finally {
    child.stdin.destroy();
    if (child.exitCode === null) child.kill('SIGTERM');
    await rm(root, { recursive: true, force: true });
  }
});
