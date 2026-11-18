import { StringDecoder } from 'node:string_decoder';

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
  let idle = null;
  let active = 0;
  let held = false;
  let serving = false;
  let closed = false;
  let stopReading = null;

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
    stopReading?.();
    input?.off?.('error', close);
    output?.off?.('error', close);
    onClose?.();
  };

  input?.on?.('error', close);
  output?.on?.('error', close);

  const armIdle = () => {
    clearIdle();
    if (!serving || closed || held || active > 0) return;
    idle = setTimeout(() => {
      idle = null;
      if (closed || held || active > 0) return;
      stopReading?.();
      input?.destroy?.();
    }, idleMs);
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
    if (closed) {
      close();
      return;
    }
    serving = true;
    armIdle();
    const inFlight = new Set();
    let chain = Promise.resolve();
    let fatalError = null;

    const dispatchLine = (rawLine, wasOversized) => {
      clearIdle();
      const line = rawLine.endsWith('\r') ? rawLine.slice(0, -1) : rawLine;
      const parsed = wasOversized ? { ok: false, reason: 'input_too_large' } : parseBridgeLine(line, lineLimit);
      if (concurrent) {
        if (!parsed.ok) {
          let invalidPromise;
          try {
            invalidPromise = Promise.resolve(onInvalid?.(parsed));
          } catch (error) {
            invalidPromise = Promise.reject(error);
          }
          inFlight.add(invalidPromise);
          void invalidPromise.catch(error => {
            if (!closed && fatalError === null) fatalError = error;
          }).finally(() => {
            inFlight.delete(invalidPromise);
            armIdle();
          });
          return;
        }
        active++;
        clearIdle();
        let pending;
        try {
          pending = Promise.resolve(onCommand(parsed.command));
        } catch (error) {
          pending = Promise.reject(error);
        }
        inFlight.add(pending);
        void pending.catch(() => undefined).finally(() => {
          inFlight.delete(pending);
          active--;
          armIdle();
        });
      } else {
        chain = chain.then(async () => {
          if (closed || fatalError !== null) return;
          clearIdle();
          if (!parsed.ok) {
            const invalidResult = onInvalid?.(parsed);
            if (invalidResult && typeof invalidResult.then === 'function') await invalidResult;
            armIdle();
            return;
          }
          active++;
          clearIdle();
          try {
            await onCommand(parsed.command);
          } finally {
            active--;
            armIdle();
          }
        }).catch(error => {
          if (!closed && fatalError === null) fatalError = error;
          stopReading?.();
        });
      }
    };

    try {
      await new Promise(resolve => {
        const decoder = new StringDecoder('utf8');
        let buffer = '';
        let oversized = false;
        let finished = false;

        const finish = flushTrailing => {
          if (finished) return;
          finished = true;
          stopReading = null;
          input?.off?.('data', onData);
          input?.off?.('end', onEnd);
          input?.off?.('close', onStreamClose);
          if (flushTrailing) {
            const tail = decoder.end();
            if (tail) appendText(tail);
            if (oversized || buffer.length > 0) {
              dispatchLine(buffer, oversized);
              buffer = '';
              oversized = false;
            }
          }
          resolve();
        };

        const appendSegment = segment => {
          if (oversized) return;
          if (buffer.length + segment.length > lineLimit) {
            oversized = true;
            buffer = '';
          } else {
            buffer += segment;
          }
        };

        const appendText = text => {
          let start = 0;
          while (start < text.length) {
            const newline = text.indexOf('\n', start);
            if (newline < 0) {
              appendSegment(text.slice(start));
              break;
            }
            appendSegment(text.slice(start, newline));
            const line = buffer;
            const lineOversized = oversized;
            buffer = '';
            oversized = false;
            dispatchLine(line, lineOversized);
            start = newline + 1;
          }
        };

        const onData = chunk => {
          if (finished || closed) return;
          appendText(typeof chunk === 'string' ? chunk : decoder.write(chunk));
        };
        const onEnd = () => finish(true);
        const onStreamClose = () => finish(false);

        stopReading = () => finish(false);
        if (input.destroyed || (input.readableEnded && input.readableLength === 0)) {
          finish(false);
          return;
        }
        input.on('end', onEnd);
        input.on('close', onStreamClose);
        input.on('data', onData);
      });

      await chain;
      if (inFlight.size > 0) await Promise.allSettled([...inFlight]);
      if (fatalError !== null && !closed) throw fatalError;
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
