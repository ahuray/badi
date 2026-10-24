import { lstatSync, openSync, readFileSync, fstatSync, closeSync, writeFileSync, writeSync, renameSync, unlinkSync, constants } from 'node:fs';
import { join, isAbsolute } from 'node:path';
import { randomUUID } from 'node:crypto';

// Editors record activity on every broker frame and keystroke, on their UI
// thread. While debugging is off, the disabled state is remembered and the
// control file is looked for at most this often; while it is on, every event
// rechecks it, so `badi debug off` stops recording at once.
const DISABLED_RECHECK_MS = 1000;
const runs = new Map();
let disabledSince = null;
const APP_IDS = Object.freeze({ obsidian: 'obsidian', terminal: 'bash', neovim: 'nvim' });

// Writers stop appending once the shared trace reaches this size.
const MAX_TRACE_BYTES = 16 * 1024 * 1024;

// One line in the shared `badi debug` trace. Only `badi debug on` creates the
// trace and `badi debug off` deletes it, so a missing file means no trace.
function appendTrace(directory, line) {
  // Test runs set BADI_DEBUG_TRACE=off so they never write into a live run.
  if (process.env.BADI_DEBUG_TRACE === 'off') return;
  let fd;
  try {
    fd = openSync(join(directory, 'debug-trace.jsonl'),
      constants.O_WRONLY | constants.O_APPEND | constants.O_NOFOLLOW);
    const metadata = fstatSync(fd);
    if (!metadata.isFile() || metadata.uid !== process.getuid() || (metadata.mode & 0o077) ||
        metadata.size > MAX_TRACE_BYTES) return;
    // One write per line: O_APPEND keeps concurrent writers' lines whole.
    writeSync(fd, `${JSON.stringify(line)}\n`);
  } catch {
    // The trace is optional diagnostics.
  } finally {
    if (fd !== undefined) closeSync(fd);
  }
}

export function recordActivity(adapter, event, reason) {
  if (!Object.hasOwn(APP_IDS, adapter) || !/^[a-z_. ]{1,64}$/.test(event) ||
      !/^[a-z_. ]{1,64}$/.test(reason)) return;
  const now = Date.now();
  if (disabledSince !== null && now >= disabledSince && now - disabledSince < DISABLED_RECHECK_MS) return;
  disabledSince = now;
  const runtime = process.env.XDG_RUNTIME_DIR;
  if (!runtime || !isAbsolute(runtime)) return;
  const directory = join(runtime, 'badi');
  let fd;
  let temporary;
  try {
    const info = lstatSync(directory);
    if (!info.isDirectory() || info.uid !== process.getuid() || (info.mode & 0o077)) return;
    fd = openSync(join(directory, 'debug-control.json'), constants.O_RDONLY | constants.O_NOFOLLOW);
    const metadata = fstatSync(fd);
    if (!metadata.isFile() || metadata.uid !== process.getuid() || (metadata.mode & 0o077) || metadata.size > 512) return;
    const control = JSON.parse(readFileSync(fd, 'utf8'));
    const seconds = Math.floor(now / 1000);
    if (!Number.isSafeInteger(control.expires_at) || control.expires_at <= seconds || control.expires_at > seconds + 900 ||
        typeof control.id !== 'string' || !/^[0-9a-f-]{36}$/.test(control.id)) return;
    disabledSince = null;
    let run = runs.get(adapter);
    if (run?.id !== control.id) {
      run = { id: control.id, counts: {}, reason_counts: {} };
      runs.set(adapter, run);
    }
    appendTrace(directory, { t: now, src: adapter, ev: event, reason });
    run.counts[event] = (run.counts[event] ?? 0) + 1;
    run.reason_counts[reason] = (run.reason_counts[reason] ?? 0) + 1;
    temporary = join(directory, `debug-${adapter}.${randomUUID()}.tmp`);
    writeFileSync(temporary, JSON.stringify({ schema: 'badi.editor-activity.v1', ...run, at: seconds,
      event, reason, app_id: APP_IDS[adapter], pid: process.pid }), { mode: 0o600, flag: 'wx' });
    renameSync(temporary, join(directory, `debug-${adapter}.json`));
    temporary = null;
  } catch (error) {
    if (!['ENOENT', 'ELOOP'].includes(error.code)) runs.delete(adapter);
  } finally {
    if (fd !== undefined) closeSync(fd);
    if (temporary) { try { unlinkSync(temporary); } catch {} }
  }
}
