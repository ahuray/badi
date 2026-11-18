import { spawn } from 'node:child_process';
import { lstatSync } from 'node:fs';
import { createConnection } from 'node:net';

// Starts the user's disabled badi-broker.service when an editor is about to
// ask the model. The session helper must not call this: its reconnect loop
// would load the model at login. Unit tests run under node:test and never
// start the session broker.

const COMMAND = ['/usr/bin/systemctl', '--user', 'start', '--no-block', 'badi-broker.service'];
const state = { last: null };

export function liveSessionRuntime(runtime, uid) {
  return typeof runtime === 'string' && Number.isInteger(uid) && runtime === `/run/user/${uid}`;
}

export function privateRuntimeMarker(path, uid = typeof process.getuid === 'function' ? process.getuid() : undefined) {
  try {
    const metadata = lstatSync(path);
    return metadata.isFile() && (uid === undefined || metadata.uid === uid) && (metadata.mode & 0o777) === 0o600;
  } catch {
    return false;
  }
}

export function brokerSocketReady(path, uid = typeof process.getuid === 'function' ? process.getuid() : undefined) {
  try {
    const metadata = lstatSync(path);
    return metadata.isSocket() && (uid === undefined || metadata.uid === uid) && (metadata.mode & 0o777) === 0o600;
  } catch {
    return false;
  }
}

function defaultLiveSocket(path, uid) {
  if (!brokerSocketReady(path, uid)) return Promise.resolve(false);
  return new Promise(resolve => {
    let settled = false;
    const finish = ok => {
      if (settled) return;
      settled = true;
      socket.destroy();
      resolve(ok);
    };
    const socket = createConnection(path);
    socket.setTimeout(500, () => finish(true));
    socket.once('connect', () => finish(true));
    socket.once('error', error => finish(error?.code !== 'ECONNREFUSED' && error?.code !== 'ENOENT'));
  });
}

function defaultSpawn(command) {
  const child = spawn(command[0], command.slice(1), { detached: true, stdio: 'ignore' });
  child.on('error', () => {});
  child.unref();
}

export async function requestBroker({
  env = process.env,
  uid = typeof process.getuid === 'function' ? process.getuid() : undefined,
  socketReady = null,
  now = Date.now,
  spawnImpl = defaultSpawn,
  slot = state,
  waitMs = 0,
} = {}) {
  if (env.NODE_TEST_CONTEXT || !liveSessionRuntime(env.XDG_RUNTIME_DIR, uid)) return false;
  const dir = `${env.XDG_RUNTIME_DIR}/badi`;
  const blocked = () => privateRuntimeMarker(`${dir}/service-stopped`, uid) ||
    privateRuntimeMarker(`${dir}/service-failed`, uid);
  if (blocked()) return false;
  const path = `${dir}/broker.sock`;
  const checkReady = socketReady ? candidate => Promise.resolve(socketReady(candidate, uid))
    : candidate => defaultLiveSocket(candidate, uid);
  if (await checkReady(path)) return true;
  const at = now();
  if (slot.last === null || slot.last === undefined || at - slot.last >= 5000) {
    slot.last = at;
    spawnImpl(COMMAND);
  }
  if (waitMs <= 0) return false;
  const deadline = at + waitMs;
  while (now() < deadline) {
    if (blocked()) return false;
    if (await checkReady(path)) return true;
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  return blocked() ? false : Boolean(await checkReady(path));
}
