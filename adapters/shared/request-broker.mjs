import { spawn } from 'node:child_process';
import { lstatSync } from 'node:fs';

// Starts the user's disabled badi-broker.service when an editor is about to
// ask the model. The session helper must not call this: its reconnect loop
// would load the model at login. Unit tests run under node:test and never
// start the session broker.

const COMMAND = ['/usr/bin/systemctl', '--user', 'start', '--no-block', 'badi-broker.service'];
const state = { last: null };

export function liveSessionRuntime(runtime, uid) {
  return typeof runtime === 'string' && Number.isInteger(uid) && runtime === `/run/user/${uid}`;
}

export function brokerSocketReady(path, uid = typeof process.getuid === 'function' ? process.getuid() : undefined) {
  try {
    const metadata = lstatSync(path);
    return metadata.isSocket() && (uid === undefined || metadata.uid === uid) && (metadata.mode & 0o777) === 0o600;
  } catch {
    return false;
  }
}

function defaultSpawn(command) {
  const child = spawn(command[0], command.slice(1), { detached: true, stdio: 'ignore' });
  child.on('error', () => {});
  child.unref();
}

export function requestBroker({
  env = process.env,
  uid = typeof process.getuid === 'function' ? process.getuid() : undefined,
  socketReady = brokerSocketReady,
  now = Date.now,
  spawnImpl = defaultSpawn,
  slot = state,
  waitMs = 0,
} = {}) {
  if (env.NODE_TEST_CONTEXT || !liveSessionRuntime(env.XDG_RUNTIME_DIR, uid)) return Promise.resolve(false);
  const path = `${env.XDG_RUNTIME_DIR}/badi/broker.sock`;
  if (socketReady(path, uid)) return Promise.resolve(true);
  const at = now();
  if (slot.last === null || slot.last === undefined || at - slot.last >= 5000) {
    slot.last = at;
    spawnImpl(COMMAND);
  }
  if (waitMs <= 0) return Promise.resolve(false);
  const deadline = at + waitMs;
  const wait = async () => {
    while (now() < deadline) {
      if (socketReady(path, uid)) return true;
      await new Promise(resolve => setTimeout(resolve, 200));
    }
    return socketReady(path, uid);
  };
  return wait();
}
