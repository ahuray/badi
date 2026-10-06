import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createInterface } from 'node:readline';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readFile, readlink, open, stat } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';
import { assertModelArtifactIdentity, assertModelArtifactUnchanged, modelArtifactArgs } from './model-artifact.mjs';

const binary = fileURLToPath(new URL('../../../target/release/badi-writing-lab', import.meta.url));

export function parseProcessStat(pid, value) {
  if (!Number.isSafeInteger(pid) || pid <= 1 || pid > 0xffffffff) throw new Error('Invalid model process PID.');
  const end = typeof value === 'string' ? value.lastIndexOf(')') : -1;
  if (typeof value !== 'string' || !value.startsWith(`${pid} (`) || end < 0) throw new Error('Malformed model process stat.');
  const parts = value.slice(end + 2).trim().split(/\s+/u);
  if (!/^[RSDZTtXxKWPI]$/u.test(parts[0] ?? '') || !/^\d+$/u.test(parts[1] ?? '')
    || !/^\d+$/u.test(parts[2] ?? '') || !/^[1-9]\d*$/u.test(parts[19] ?? '')
    || !Number.isSafeInteger(Number(parts[1])) || !Number.isSafeInteger(Number(parts[2]))) {
    throw new Error('Malformed model process stat.');
  }
  return { pid, parent: Number(parts[1]), group: Number(parts[2]), start: parts[19] };
}

const executableKey = metadata => [metadata.dev, metadata.ino, metadata.size, metadata.mtimeNs, metadata.ctimeNs].join(':');

export async function processIdentity(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 1 || pid > 0xffffffff) throw new Error('Invalid model process PID.');
  try {
    const identity = parseProcessStat(pid, await readFile(`/proc/${pid}/stat`, 'utf8'));
    return { ...identity, executable: await readlink(`/proc/${pid}/exe`),
      executable_key: executableKey(await stat(`/proc/${pid}/exe`, { bigint: true })) };
  } catch (error) {
    if (error.code === 'ENOENT' || error.code === 'ESRCH') return null;
    throw new Error('Could not inspect the model process identity.', { cause: error });
  }
}

async function verifyRuntime(message, workerPid, cache, prefillBatch) {
  if (message.schema !== 'badi.prediction-lab.worker.v1' || !message.identity || typeof message.identity !== 'object'
    || Array.isArray(message.identity) || typeof message.identity.binary_sha256 !== 'string'
    || !/^[a-f0-9]{64}$/u.test(message.identity.binary_sha256)) throw new Error('Invalid verified runtime identity.');
  if (message.identity.batch_size !== prefillBatch || message.identity.ubatch_size !== prefillBatch) {
    throw new Error('The model runtime does not match the selected prompt-processing batch.');
  }
  const runtime = await processIdentity(message.runtime_pid);
  if (!runtime || runtime.parent !== workerPid || runtime.group !== runtime.pid
    || !runtime.executable.endsWith('/llama-server')) throw new Error('Model runtime ownership does not match its worker.');
  const file = await open(`/proc/${runtime.pid}/exe`, 'r');
  let digest;
  try {
    const key = executableKey(await file.stat({ bigint: true }));
    if (key !== runtime.executable_key) throw new Error('The model executable changed during inspection.');
    digest = cache.get(key);
    if (!digest) {
      const hash = createHash('sha256');
      for await (const chunk of file.createReadStream({ autoClose: false })) hash.update(chunk);
      digest = hash.digest('hex');
    }
    if (executableKey(await file.stat({ bigint: true })) !== key) throw new Error('The model executable changed during hashing.');
    if (digest !== message.identity.binary_sha256) throw new Error('The model executable differs from its verified identity.');
    cache.set(key, digest);
  } finally { await file.close(); }
  const current = await processIdentity(runtime.pid);
  if (!current || current.parent !== workerPid || current.group !== runtime.pid || current.start !== runtime.start
    || current.executable !== runtime.executable || current.executable_key !== runtime.executable_key) {
    throw new Error('The model runtime changed before readiness.');
  }
  return { ...runtime, executable_sha256: digest };
}

async function cleanupRuntime(identity) {
  if (!identity) return;
  const stillOwned = async () => {
    const current = await processIdentity(identity.pid);
    if (!current || current.start !== identity.start) return false;
    if (current.executable !== identity.executable || current.executable_key !== identity.executable_key || current.group !== identity.pid) {
      throw new Error('The owned model process identity changed during cleanup.');
    }
    return true;
  };
  // Graceful Rust cleanup normally did this already. If its worker crashed,
  // identify the exact runtime we observed as its child before signalling it.
  if (!await stillOwned()) return;
  try { process.kill(-identity.pid, 'SIGTERM'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
  for (let i = 0; i < 20 && await stillOwned(); i++) await delay(100);
  if (await stillOwned()) {
    try { process.kill(-identity.pid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
    for (let i = 0; i < 10 && await stillOwned(); i++) await delay(100);
  }
  if (await stillOwned()) throw new Error('Owned model process did not stop.');
}

// The browser never selects an executable, model path, runtime address or key.
export class LabWorker {
  constructor({ executable = binary, args = [], startupMs = 30000, prefillBatch = 16 } = {}) {
    if (![16, 64].includes(prefillBatch) || !Array.isArray(args)
      || args.some(arg => typeof arg !== 'string' || arg === '--prefill-batch' || arg.startsWith('--prefill-batch='))) {
      throw new Error('Select prefillBatch 16 or 64 through the worker option, not raw arguments.');
    }
    this.executable = resolve(executable);
    this.args = [...args, ...(prefillBatch === 64 ? ['--prefill-batch', '64'] : [])];
    this.originalArgs = [...this.args];
    this.prefillBatch = prefillBatch;
    this.startupMs = startupMs;
    this.session = null;
    this.runtimeExecutableCache = new Map();
    this.generation = 0;
  }

  async selectArtifact(artifact, beforeStart) {
    await this.stop();
    this.modelArtifact = artifact;
    this.beforeStart = beforeStart;
    this.args = [...this.originalArgs, ...modelArtifactArgs(artifact)];
  }

  async start() {
    if (this.cleanupError) throw this.cleanupError;
    if (this.session?.closed) { await this.session.exit; return this.start(); }
    if (this.session) return this.session.ready;
    const startupStarted = performance.now();
    const child = spawn(this.executable, this.args, { stdio: ['pipe', 'pipe', 'ignore'] });
    const session = { child, pending: null, identity: null, closed: false };
    this.session = session;
    session.exit = new Promise(resolveExit => child.once('close', async () => {
      clearInterval(session.resourceTimer);
      try { await session.captureRuntime; await cleanupRuntime(session.runtime); }
      catch {
        session.cleanupError = new Error('Could not confirm cleanup of the owned model process.');
        session.fail(session.cleanupError);
        this.cleanupError = session.cleanupError;
      }
      finally { if (this.session === session) this.session = null; resolveExit(); }
    }));
    session.ready = new Promise((resolveReady, rejectReady) => {
      session.fail = error => {
        rejectReady(error);
        session.pending?.reject(error);
        session.pending = null;
      };
      child.once('error', () => session.fail(new Error('Cannot start the model worker. Run npm run writing:lab to build it.')));
      child.once('close', () => {
        session.closed = true;
        clearTimeout(session.timer);
        session.fail(new Error('The model worker stopped. Run again to start a fresh session.'));
      });
      const lines = createInterface({ input: child.stdout });
      lines.once('close', () => {
        if (!session.closed && !session.stopping) {
          session.fail(new Error('The model response stream closed. Run again to start a fresh session.'));
          void this.stop().catch(() => {}); // Cleanup failures are latched and rethrown by the awaited owner.
        }
      });
      let bytes = 0;
      child.stdout.on('data', chunk => {
        bytes += chunk.length;
        if (bytes > 1024 * 1024) { session.fail(new Error('Model response exceeded its limit.')); child.kill('SIGTERM'); }
      });
      child.stdin.on('error', () => { session.fail(new Error('The model input stream closed.')); void this.stop().catch(() => {}); });
      lines.on('line', line => {
        bytes = 0;
        let message;
        try {
          message = JSON.parse(line);
          if (!message || typeof message !== 'object' || Array.isArray(message)) throw new Error();
        } catch { session.fail(new Error('Invalid model response.')); child.kill('SIGTERM'); return; }
        if (message.type === 'ready' && !session.readyAnnounced) {
          session.readyAnnounced = true;
          session.captureRuntime = (async () => {
            try {
              session.runtime = await verifyRuntime(message, child.pid, this.runtimeExecutableCache, this.prefillBatch);
              assertModelArtifactIdentity(message.identity, this.modelArtifact);
              if (session.closed || session.stopping) return;
              session.identity = message.identity;
              session.startupMs = performance.now() - startupStarted;
              session.peakRss = 0;
              session.sampleResource = async () => {
                try {
                  const status = await readFile(`/proc/${session.runtime.pid}/status`, 'utf8');
                  const rss = Number(status.match(/^VmHWM:\s+(\d+) kB$/mu)?.[1]);
                  if (Number.isFinite(rss) && rss > 0) session.peakRss = Math.max(session.peakRss, rss * 1024);
                } catch { /* A reaped process has no sample; absence is not zero usage. */ }
              };
              session.resourceTimer = setInterval(() => { void session.sampleResource(); }, 100);
              session.resourceTimer.unref();
              clearTimeout(session.timer);
              resolveReady(message.identity);
            } catch (error) {
              session.cleanupError = new Error('Could not verify ownership and cleanup of the model runtime.', { cause: error });
              this.cleanupError = session.cleanupError;
              session.fail(session.cleanupError);
              void this.stop().catch(() => {});
            }
          })();
        } else if (message.type === 'stopped' && session.stopping) {
          session.cleanup = message.cleanup;
        } else if (message.type === 'error' && !session.identity) {
          session.fail(new Error(`Model startup failed: ${message.error}`));
          child.kill('SIGTERM');
        } else if (session.pending && message.id === session.pending.id) {
          session.pending.resolve(message);
          session.pending = null;
        } else {
          session.fail(new Error('Unexpected model response identity.'));
          child.kill('SIGTERM');
        }
      });
      session.timer = setTimeout(() => { session.fail(new Error('Model startup timed out.')); child.kill('SIGTERM'); }, this.startupMs);
    });
    return session.ready;
  }

  async predict(request, signal) {
    signal?.throwIfAborted();
    if (this.preparing) throw new Error('A model request is already running.');
    const generation = this.generation;
    const abort = () => { void this.stop().catch(() => {}); };
    signal?.addEventListener('abort', abort, { once: true });
    try {
      if (!this.session && (this.beforeStart || this.modelArtifact)) {
        this.preparing = true;
        try {
          await this.beforeStart?.(signal);
          await assertModelArtifactUnchanged(this.modelArtifact);
          signal?.throwIfAborted();
          if (generation !== this.generation) throw new Error('Model preparation was cancelled.');
        } finally { this.preparing = false; }
      }
      await this.start();
      signal?.throwIfAborted();
      const session = this.session;
      if (!session || session.pending) throw new Error('A model request is already running.');
      const requestStarted = performance.now();
      return await new Promise((resolveResult, rejectResult) => {
        const timer = setTimeout(() => {
          rejectResult(new Error('Model request timed out.'));
          void this.stop().catch(() => {});
        }, request.config.budget_ms + 15000);
        session.pending = { id: request.id,
          resolve: result => { clearTimeout(timer); resolveResult({ ...result, local_measurement: {
            worker_startup_ms: session.startupMs, request_roundtrip_ms: performance.now() - requestStarted,
            peak_runtime_rss_bytes: session.peakRss || null,
            gpu_memory_bytes: session.identity.gpu_layers === 0 ? 0 : null,
            memory_measurement: 'Linux VmHWM for this owned runtime; shared pages may also appear in other processes. GPU zero only for CPU-only launch.',
          } }); },
          reject: error => { clearTimeout(timer); rejectResult(error); } };
        session.child.stdin.write(JSON.stringify(request) + '\n', error => {
          if (error) session.pending?.reject(new Error('Could not send a request to the model.'));
        });
      });
    } finally { signal?.removeEventListener('abort', abort); }
  }

  async stop() {
    this.generation++;
    const session = this.session;
    if (!session) { if (this.cleanupError) throw this.cleanupError; return; }
    if (session.stopping) return session.stopping;
    session.stopping = (async () => {
      clearInterval(session.resourceTimer);
      session.fail(new Error('Model session cancelled.'));
      session.child.kill('SIGTERM');
      const force = setTimeout(() => session.child.kill('SIGKILL'), 6000);
      await session.exit;
      clearTimeout(force);
      if (this.session === session) this.session = null;
      if (session.cleanupError) throw session.cleanupError;
      return session.cleanup ?? null;
    })();
    return session.stopping;
  }
}
