import { createInterface } from 'node:readline';

// Shared newline-delimited stdio and idle-exit lifecycle for the Bash and
// Neovim editor bridges. Each bridge exits after ten quiet minutes unless a
// command is in flight or the editor holds terminal focus.

export const DEFAULT_IDLE_MS = 10 * 60 * 1000;
export const LINE_LIMIT = 16384;

const guardedStreams = new WeakSet();

function guardStreamErrors(stream) {
  if (stream && typeof stream.on === 'function' && !guardedStreams.has(stream)) {
    guardedStreams.add(stream);
    stream.on('error', () => {});
  }
}

export function bridgeIdleMs(env = process.env) {
  const value = env?.BADI_BRIDGE_IDLE_MS ?? '';
  return /^[1-9][0-9]{0,8}$/.test(value) ? Number(value) : DEFAULT_IDLE_MS;
}

export function parseBridgeLine(line, lineLimit = LINE_LIMIT) {
  if (typeof line !== 'string' || line.length > lineLimit) {
    return { ok: false, reason: 'input_too_large' };
  }
  let command;
  try {
    command = JSON.parse(line);
  } catch {
    return { ok: false, reason: 'invalid_json' };
  }
  if (command === null) return { ok: false, reason: 'invalid_json' };
  if (typeof command !== 'object' || Array.isArray(command)) {
    return { ok: false, reason: 'invalid_object' };
  }
  return { ok: true, command };
}

export function createBridgeStdio({
  input = process.stdin,
  output = process.stdout,
  env = process.env,
  idleMs = bridgeIdleMs(env),
  lineLimit = LINE_LIMIT,
  concurrent = false,
  onClose = null,
} = {}) {
  guardStreamErrors(input);
  guardStreamErrors(output);
  const lines = createInterface({ input, terminal: false });
  let linesClosed = false;
  lines.once('close', () => { linesClosed = true; });
  let idle = null;
  let active = 0;
  let held = false;
  let serving = false;
  let closed = false;

  const clearIdle = () => {
    if (idle !== null) {
      clearTimeout(idle);
      idle = null;
    }
  };

  const close = () => {
    if (closed) return;
    closed = true;
    serving = false;
    clearIdle();
    input?.off?.('error', close);
    output?.off?.('error', close);
    try {
      onClose?.();
    } finally {
      lines.close();
    }
  };

  input?.on?.('error', close);
  output?.on?.('error', close);

  const armIdle = () => {
    clearIdle();
    if (!serving || closed || held || active > 0) return;
    idle = setTimeout(() => {
      idle = null;
      if (closed || held || active > 0) return;
      lines.close();
      input?.destroy?.();
    }, idleMs);
    idle.unref?.();
  };

  const writeLine = text => {
    if (output.destroyed || output.writableEnded) return false;
    return output.write(`${text}\n`);
  };

  const writeJson = message => writeLine(JSON.stringify(message));

  const setHeld = value => {
    held = value === true;
    armIdle();
  };

  const serve = async ({ onCommand, onInvalid } = {}) => {
    if (closed || linesClosed) {
      close();
      return;
    }
    serving = true;
    armIdle();
    try {
      for await (const line of lines) {
        clearIdle();
        const parsed = parseBridgeLine(line, lineLimit);
        if (!parsed.ok) {
          const invalidResult = onInvalid?.(parsed);
          if (invalidResult && typeof invalidResult.then === 'function') await invalidResult;
          armIdle();
          continue;
        }
        active++;
        clearIdle();
        if (concurrent) {
          let pending;
          try {
            pending = Promise.resolve(onCommand(parsed.command));
          } catch (error) {
            pending = Promise.reject(error);
          }
          void pending.catch(() => undefined).finally(() => {
            active--;
            armIdle();
          });
        } else {
          try {
            await onCommand(parsed.command);
          } finally {
            active--;
            armIdle();
          }
        }
      }
    } catch (error) {
      if (!closed) throw error;
    } finally {
      close();
    }
  };

  return {
    get active() { return active; },
    get closed() { return closed; },
    get held() { return held; },
    get idleArmed() { return idle !== null; },
    idleMs,
    lineLimit,
    writeLine,
    writeJson,
    setHeld,
    serve,
    close,
  };
}
