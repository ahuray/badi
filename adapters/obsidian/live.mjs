import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, mkdir, writeFile, readFile, cp, rm, lstat } from 'node:fs/promises';
import { once } from 'node:events';
import { resolve, join } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { chromium } from 'playwright';

// Opt-in: the actual Obsidian application, a disposable vault/profile and broker.
// CDP drives only this process. No existing vault, plugin or service is changed.
const ROOT = resolve(import.meta.dirname, '../..');
const local = process.argv.includes('--local');
const headless = process.argv.includes('--headless');
const run = promisify(execFile);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let previousWindow, ownedAddress;
async function unlockedDesktop() {
  const lock = JSON.parse((await run('omarchy-shell', ['lock', 'status'])).stdout);
  assert.ok(['locked', 'requested', 'pending', 'sessionLocked', 'secure'].every(key => lock[key] === false),
    'Run the visible Obsidian trial after normal desktop unlock');
}
async function focusWindow(address) {
  assert.match(address, /^0x[0-9a-f]+$/u);
  await run('hyprctl', ['dispatch', `hl.dsp.focus({ window = "address:${address}" })`]);
}
if (!headless) {
  assert.ok(process.env.XDG_RUNTIME_DIR && process.env.WAYLAND_DISPLAY, 'The visible trial needs the Wayland session');
  await unlockedDesktop();
  assert.ok(JSON.parse((await run('hyprctl', ['monitors', '-j'])).stdout).some(monitor => monitor.dpmsStatus),
    'The visible trial requires an already awake display');
  previousWindow = JSON.parse((await run('hyprctl', ['activewindow', '-j'])).stdout).address;
}
const root = await mkdtemp('/tmp/badi-obsidian-live-');
await mkdir(join(ROOT, 'output/playwright'), { recursive: true });
const output = await mkdtemp(join(ROOT, 'output/playwright/obsidian-'));
const profile = join(root, 'profile');
const vault = join(root, 'Badi writing trial');
const env = { ...process.env, HOME: root, XDG_RUNTIME_DIR: join(root, 'runtime'),
  XDG_CONFIG_HOME: join(root, 'config'), XDG_DATA_HOME: join(root, 'data'),
  XDG_CACHE_HOME: join(root, 'cache') };
// Headless Ozone uses no display; the window finds the session's Wayland
// socket by absolute path, since XDG_RUNTIME_DIR is the trial's own.
if (headless) { delete env.WAYLAND_DISPLAY; delete env.DISPLAY; }
else env.WAYLAND_DISPLAY = resolve(process.env.XDG_RUNTIME_DIR, process.env.WAYLAND_DISPLAY);
const endpoint = join(env.XDG_RUNTIME_DIR, 'badi/broker.sock');
const control = async (...args) => JSON.parse((await run(join(ROOT, 'target/release/badictl'),
  ['--socket', endpoint, ...args], { env, timeout: 5000 })).stdout);
const report = { provider: local ? 'local_model' : 'phrase_fixture', success: false, samples: [],
  started_at: new Date().toISOString(),
  mid_line: local,
  boundary: `Actual Obsidian/Electron with ${headless ? 'headless Ozone' : 'a Wayland window'}, isolated vault/profile and real broker; synthetic text. CDP drives keys and editor setup, not compositor shortcut routing or physical keys. This lane does not measure writing usefulness or visible latency.`,
  compositor_captures: [] };
let broker, app, browser, page;
let brokerLog = '', appLog = '';
async function ownedWindow() {
  if (headless) return null;
  await unlockedDesktop();
  const active = JSON.parse((await run('hyprctl', ['activewindow', '-j'])).stdout);
  const title = await page.title();
  assert.ok(Number.isSafeInteger(active.pid) && active.pid > 0, 'The private Obsidian window must be active');
  // The installed launcher execs Electron, so the spawned process owns the
  // private window even when Electron rewrites its process title/argv.
  assert.equal(app.exitCode, null, 'The owned Obsidian process must still be running');
  assert.equal(active.pid, app.pid, 'Stop after another window gains focus');
  assert.equal(active.xwayland, false, 'The visible trial must use native Wayland');
  assert.equal(active.title, title, 'Only the CDP target editor window may be captured');
  if (ownedAddress) assert.equal(active.address, ownedAddress);
  else ownedAddress = active.address;
  // Record titles only after proving this is the disposable app's window.
  report.window_guard = { app_pid: app.pid, address: active.address, pid: active.pid,
    class: active.class, title: active.title, target_title: title };
  return active;
}
async function press(key) {
  await ownedWindow();
  await page.keyboard.press(key);
}
async function startBroker() {
  broker = spawn(join(ROOT, 'target/release/badi-broker'), local
    ? ['--provider', 'local', '--model-directory', join(process.env.HOME, '.local/share/badi')]
    : ['--provider', 'phrase'], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  broker.stdout.on('data', data => { brokerLog += data; });
  broker.stderr.on('data', data => { brokerLog += data; });
  for (let attempt = 0; ; attempt++) {
    assert.equal(broker.exitCode, null, brokerLog);
    if (await lstat(endpoint).then(() => true, () => false)) break;
    if (attempt >= 300) throw new Error('Private broker did not become ready');
    await delay(100);
  }
  assert.equal((await control('status')).provider, local ? 'local_model' : 'phrase_v1');
}
async function stopBroker() {
  if (!broker || broker.exitCode !== null) return;
  const exited = once(broker, 'exit');
  broker.kill('SIGTERM');
  const timer = setTimeout(() => broker.kill('SIGKILL'), 5000);
  try { await exited; } finally { clearTimeout(timer); }
}
try {
  for (const path of [profile, vault, env.XDG_RUNTIME_DIR, env.XDG_CACHE_HOME,
    join(vault, '.obsidian/plugins/badi'), join(env.XDG_CONFIG_HOME, 'badi')]) {
    await mkdir(path, { recursive: true, mode: 0o700 });
  }
  await writeFile(join(env.XDG_CONFIG_HOME, 'badi/settings.json'), JSON.stringify({
    schema: 'badi.settings.v2', revision: 1, paused: false, ...(local ? { mid_line: true } : {}), subjects: [{
      identity: { kind: 'linux_app', adapter: 'obsidian', app_id: 'obsidian' },
      permissions: { context_read: 'allow', display: 'allow', suggest: 'allow', learn: 'block', retention: { mode: 'none' } },
    }],
  }), { mode: 0o600 });
  await run('node', ['adapters/obsidian/build.mjs'], { cwd: ROOT });
  for (const name of ['main.js', 'manifest.json', 'styles.css']) {
    await cp(join(ROOT, 'adapters/obsidian/dist', name), join(vault, '.obsidian/plugins/badi', name));
  }
  await writeFile(join(profile, 'obsidian.json'), JSON.stringify({ vaults: {
    baditrial: { path: vault, ts: Date.now(), open: true },
  } }));
  await writeFile(join(vault, '.obsidian/community-plugins.json'), '["badi"]');
  await writeFile(join(vault, '.obsidian/core-plugins.json'), '[]');
  await writeFile(join(vault, '.obsidian/app.json'), JSON.stringify({ livePreview: true, defaultViewMode: 'source' }));
  await writeFile(join(vault, 'Trial.md'), '');
  await startBroker();
  const sources = ['adapters/obsidian/main.mjs', 'adapters/obsidian/revision.mjs',
    'adapters/obsidian/styles.css', 'adapters/shared/broker-client.mjs', 'adapters/shared/revision.mjs'];
  report.source = { base_commit: (await run('git', ['rev-parse', 'HEAD'], { cwd: ROOT })).stdout.trim(),
    sha256: Object.fromEntries(await Promise.all(sources.map(async path => [path,
      createHash('sha256').update(await readFile(join(ROOT, path))).digest('hex')]))),
    broker_sha256: createHash('sha256').update(await readFile(join(ROOT, 'target/release/badi-broker'))).digest('hex') };
  report.broker_initial = await control('status');
  // Only this disposable profile's synthetic text is traced. The private
  // marker expires and the entire runtime directory is removed in finally.
  await writeFile(join(env.XDG_RUNTIME_DIR, 'badi/debug-trace.jsonl'), '', { mode: 0o600 });
  await writeFile(join(env.XDG_RUNTIME_DIR, 'badi/debug-control.json'), JSON.stringify({
    id: randomUUID(), expires_at: Math.floor(Date.now() / 1000) + 900, text: true,
  }), { mode: 0o600 });
  app = spawn('/usr/bin/obsidian', [`--user-data-dir=${profile}`, '--remote-debugging-port=0',
    '--remote-debugging-address=127.0.0.1', `--ozone-platform=${headless ? 'headless' : 'wayland'}`, '--disable-background-networking'],
  { env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  app.stdout.on('data', data => { appLog += data; });
  app.stderr.on('data', data => { appLog += data; });
  let debugging;
  for (let attempt = 0; ; attempt++) {
    assert.equal(app.exitCode, null, appLog);
    debugging = await readFile(join(profile, 'DevToolsActivePort'), 'utf8').catch(() => '');
    if (debugging) break;
    if (attempt >= 100) throw new Error(`Obsidian debugging endpoint unavailable: ${appLog}`);
    await delay(100);
  }
  const [port, path] = debugging.trim().split('\n');
  browser = await chromium.connectOverCDP(`ws://127.0.0.1:${port}${path}`);
  const context = browser.contexts()[0];
  page = context.pages()[0] ?? await context.waitForEvent('page');
  page.setDefaultTimeout(10000);
  const cdp = await context.newCDPSession(page);
  if (headless) await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 1200, height: 900, deviceScaleFactor: 1, mobile: false,
  });
  await page.waitForFunction(() => globalThis.app?.workspace?.layoutReady);
  report.versions = await page.evaluate(() => ({
    electron: process.versions.electron, chromium: process.versions.chrome }));
  report.versions.obsidian_package = (await run('pacman', ['-Q', 'obsidian'])).stdout.trim();
  await page.evaluate(async () => {
    const file = app.vault.getAbstractFileByPath('Trial.md');
    await app.workspace.getLeaf().openFile(file);
  });
  if (!headless) await unlockedDesktop();
  await page.bringToFront();
  if (!headless) {
    const windows = JSON.parse((await run('hyprctl', ['clients', '-j'])).stdout)
      .filter(window => window.pid === app.pid && !window.xwayland);
    assert.equal(windows.length, 1, 'The spawned Obsidian process must own exactly one Wayland window');
    ownedAddress = windows[0].address;
    await focusWindow(ownedAddress);
  }
  await ownedWindow();
  // Trust only this newly created vault containing the plugin built above.
  await page.getByRole('button', { name: 'Trust author and enable plugins', exact: true }).click();
  const editor = page.locator('.workspace-leaf.mod-active .cm-content');
  await editor.click();
  await page.waitForFunction(() => app.plugins.plugins.badi?.views.size > 0);
  if (!headless) {
    // Trusting this new vault opens an owned Settings window. Close only
    // that explicit first-run window before testing the editor's seat focus.
    const settingsTitle = (await page.title()).replace(/^Trial - /u, 'Settings - ');
    for (const extra of context.pages()) {
      if (extra !== page && (await extra.title()) === settingsTitle) {
        await extra.close();
      }
    }
    await unlockedDesktop();
    await focusWindow(ownedAddress);
    await ownedWindow();
  }
  const ghost = page.locator('.badi-inline');
  const text = () => page.evaluate(() => app.workspace.activeLeaf.view.editor.getValue());
  const capture = async name => {
    await ownedWindow();
    const image = await cdp.send('Page.captureScreenshot', { format: 'png' });
    await writeFile(join(output, `${name}.png`), Buffer.from(image.data, 'base64'));
    if (!headless) {
      const window = await ownedWindow();
      const geometry = `${window.at[0]},${window.at[1]} ${window.size[0]}x${window.size[1]}`;
      const file = `${name}-wayland.png`;
      await run('grim', ['-g', geometry, join(output, file)], { timeout: 10000 });
      report.compositor_captures.push({ name, file, geometry, address: window.address, pid: window.pid });
    }
  };
  const setPrefix = async prefix => {
    await ownedWindow();
    await page.evaluate(() => app.workspace.activeLeaf.view.editor.setValue(''));
    await editor.click();
    await ownedWindow();
    await page.keyboard.insertText(prefix);
  };
  const prefix = local ? 'Please find attached the' : 'thank you';
  await setPrefix(prefix);
  await ghost.waitFor({ state: 'visible' });
  const suggestion = await ghost.textContent();
  assert.ok(suggestion?.trim());
  // What Tab takes is drawn a step brighter than the rest.
  if (suggestion.trim().includes(' ')) {
    const word = page.locator('.badi-inline-word');
    report.tab_word = { word: await word.textContent(), color: await word.evaluate(element => getComputedStyle(element).color),
      rest: await ghost.evaluate(element => getComputedStyle(element).color) };
    assert.ok(suggestion.startsWith(report.tab_word.word) && report.tab_word.word.length < suggestion.length);
    assert.notEqual(report.tab_word.color, report.tab_word.rest, 'The word Tab takes has its own colour');
  }
  await capture('english');
  await delay(800);
  assert.equal(await ghost.isVisible(), true, 'The preview survives a reading pause');
  await press('Tab');
  const firstWord = /^\s*\S+/u.exec(suggestion)[0];
  await page.waitForFunction(expected => app.workspace.activeLeaf.view.editor.getValue() === expected, prefix + firstWord);
  await press('Control+z');
  assert.equal(await text(), prefix, 'One native undo removes exactly the accepted word');
  // Undo asks again. Tab with no suggestion indents the line.
  await ghost.waitFor({ state: 'visible' });
  const all = await ghost.textContent();
  await press('Control+ArrowRight');
  await page.waitForFunction(expected => app.workspace.activeLeaf.view.editor.getValue() === expected, prefix + all);
  await press('Control+z');
  assert.equal(await text(), prefix, 'Full acceptance is one native undo step');
  await ghost.waitFor({ state: 'visible' });
  await page.evaluate(() => {
    globalThis.badiEscapeTrace = [];
    const view = app.workspace.activeLeaf.view.editor.cm;
    const controller = app.plugins.plugins.badi.views.get(view);
    const dismiss = controller.dismiss.bind(controller);
    controller.dismiss = event => {
      if (event.key === 'Escape') badiEscapeTrace.push({ where: 'badi-observer', trusted: event.isTrusted,
        phase: event.eventPhase, prevented: event.defaultPrevented });
      return dismiss(event);
    };
  });
  await press('Escape');
  await ghost.waitFor({ state: 'hidden', timeout: 500 });
  report.escape_event = await page.evaluate(() => badiEscapeTrace.find(event => event.where === 'badi-observer'));
  assert.equal(report.escape_event?.trusted, true, 'The editor observer received the real Escape event');
  await delay(600);
  assert.equal(await ghost.count(), 0, 'Dismissal stays quiet until new input or explicit invocation');
  assert.equal(await text(), prefix);
  await press('Control+Shift+Space');
  await ghost.waitFor({ state: 'visible' });
  // The key again on the shown suggestion: one that starts with another word.
  const asked = await ghost.textContent();
  await press('Control+Shift+Space');
  await page.waitForFunction(previous => {
    const shown = document.querySelector('.badi-inline');
    return shown && shown.textContent !== previous;
  }, asked);
  const alternative = await ghost.textContent();
  const firstWordOf = value => /^\s*\S+/u.exec(value)?.[0].trim().toLowerCase();
  assert.notEqual(firstWordOf(alternative), firstWordOf(asked), 'An alternative starts with another word');
  report.alternative = { asked, alternative };
  await capture('alternative');
  await press('ArrowLeft');
  await ghost.waitFor({ state: 'hidden', timeout: 500 });
  await setPrefix(prefix); await ghost.waitFor({ state: 'visible' });
  await stopBroker();
  await ghost.waitFor({ state: 'hidden' });
  await startBroker();
  await ghost.waitFor({ state: 'visible', timeout: 15000 });
  const afterRestart = await ghost.textContent();
  await press('Control+ArrowRight');
  await page.waitForFunction(expected => app.workspace.activeLeaf.view.editor.getValue() === expected, prefix + afterRestart);
  await press('Control+z');
  assert.equal(await text(), prefix, 'The open note recovers after broker restart');
  if (local) {
    // Persian is off by default outside terminals since 2026-10-23.
    for (const [language, before] of [['de', 'Vielen Dank für Ihre']]) {
      await page.evaluate(language => app.commands.executeCommandById(`badi:language-${language}`), language);
      await page.waitForFunction(language => app.plugins.plugins.badi.language === language, language);
      await setPrefix(before); await ghost.waitFor({ state: 'visible' });
      const continuation = await ghost.textContent();
      await capture(language);
      await press('Control+ArrowRight');
      await page.waitForFunction(expected => app.workspace.activeLeaf.view.editor.getValue() === expected, before + continuation);
      await press('Control+z');
      assert.equal(await text(), before);
      report.samples.push({ language, prefix: before, suggestion: continuation });
    }
  }
  // Explicit selection revision in the actual editor, still against this
  // runner's private broker and disposable note. The phrase provider uses
  // the broker's own deterministic reviser; --local uses its revision model.
  await page.evaluate(() => app.commands.executeCommandById('badi:language-en'));
  await page.waitForFunction(() => app.plugins.plugins.badi.language === 'en');
  if (local) {
    // Mid-text, the editor cannot offer the provider's suffix correction.
    // Prove that a real field reports the new suppression and stays quiet.
    const before = (await control('status')).metrics.no_suggestion.typo_suspected;
    const unresolved = 'The shipping adress next';
    await setPrefix(unresolved);
    await page.evaluate(() => app.workspace.activeLeaf.view.editor.cm.dispatch({ selection: { anchor: 20 } }));
    await press('Control+Shift+Space');
    let after = before;
    for (let attempt = 0; attempt < 100 && after <= before; attempt++) {
      after = (await control('status')).metrics.no_suggestion.typo_suspected;
      if (after <= before) await delay(50);
    }
    assert.ok(after > before, 'A suspected typo without a correction path is counted');
    await ghost.waitFor({ state: 'hidden' });
    assert.equal(await text(), unresolved, 'Suppression never edits the field');
    await capture('typo-suppression');
    report.typo_suppression = { counter_before: before, counter_after: after, unchanged: true };
  }
  const select = async (note, selected, reverse = false) => {
    await ownedWindow();
    await page.evaluate(({ note, selected, reverse }) => {
      const editor = app.workspace.activeLeaf.view.editor;
      editor.setValue(note);
      const from = note.indexOf(selected), to = from + selected.length;
      editor.setSelection(editor.offsetToPos(reverse ? to : from), editor.offsetToPos(reverse ? from : to));
      editor.focus();
    }, { note, selected, reverse });
  };
  const revision = page.locator('.badi-revision');
  const waitForFullRevision = () => page.waitForFunction(() => {
    const controller = app.plugins.plugins.badi?.session;
    const element = document.querySelector('.badi-revision');
    const rect = element?.getBoundingClientRect();
    const pane = controller?.view.scrollDOM.getBoundingClientRect();
    return rect?.width > 0 && pane && rect.top >= Math.max(0, pane.top) && rect.bottom <= Math.min(innerHeight, pane.bottom)
      && rect.right <= Math.min(innerWidth, pane.right) && rect.left >= Math.max(0, pane.left)
      && element.scrollWidth <= element.clientWidth && element.scrollHeight <= element.clientHeight
      && [...element.children].every(child => { const childRect = child.getBoundingClientRect();
        return childRect.top >= rect.top && childRect.bottom <= rect.bottom; });
  });
  const note = '🙂 Before I has went. After';
  await select(note, 'I has went.', true);
  await press('Control+Shift+Space');
  await revision.waitFor({ state: 'visible', timeout: 47000 });
  await waitForFullRevision();
  const revisionText = await page.evaluate(() => app.plugins.plugins.badi.session.text);
  assert.ok(revisionText && revisionText !== 'I has went.');
  assert.equal(await text(), note, 'A marked revision never edits the original selection');
  assert.ok(await revision.locator('.badi-revision-removed').count());
  report.revision_preview = await revision.evaluate(element => ({
    rect: element.getBoundingClientRect().toJSON(),
    background: getComputedStyle(element).backgroundColor,
    color: getComputedStyle(element).color,
    children: [...element.children].map(child => ({ text: child.textContent,
      rect: child.getBoundingClientRect().toJSON(), display: getComputedStyle(child).display,
      position: getComputedStyle(child).position, color: getComputedStyle(child).color })),
  }));
  await capture('selection-revision');
  await press('Tab');
  const revised = note.replace('I has went.', revisionText);
  await page.waitForFunction(expected => app.workspace.activeLeaf.view.editor.getValue() === expected, revised);
  await press('Control+z');
  assert.equal(await text(), note, 'One native undo restores exactly the revised selection');
  report.revision = { original: 'I has went.', revised: revisionText, reverse: true, unicode_prefix: true };

  await select(note, 'I has went.');
  await press('Control+Shift+Space');
  await revision.waitFor({ state: 'visible', timeout: 47000 });
  await press('ArrowLeft');
  await revision.waitFor({ state: 'hidden' });
  assert.equal(await text(), note, 'Moving the selection only withdraws the revision');

  const synonymNote = local ? 'She will look after the garden.' : 'The big house';
  const synonymSelection = local ? 'look after' : 'big';
  if (local) {
    // The current model copies its worked example for this input. Keep
    // that failure visible as a negative test, never as a successful synonym.
    await select('The big house', 'big');
    await page.evaluate(() => app.commands.executeCommandById('badi:revise-synonyms'));
    await page.waitForFunction(() => {
      const controller = app.plugins.plugins.badi.session;
      return !controller.pending && controller.revisionReason !== null;
    }, null, { timeout: 47000 });
    const reason = await page.evaluate(() => app.plugins.plugins.badi.session.revisionReason);
    assert.equal(reason, 'invalid_output');
    await revision.waitFor({ state: 'hidden' });
    assert.equal(await text(), 'The big house');
    report.synonym_rejection = { original: 'big', reason, unchanged: true };
    await capture('synonym-rejected');
  }
  // This positive local-model case was selected from the synthetic
  // development comparison. It proves editor behavior, not synonym quality.
  await select(synonymNote, synonymSelection);
  await page.evaluate(() => app.commands.executeCommandById('badi:revise-synonyms'));
  await revision.waitFor({ state: 'visible', timeout: 47000 });
  const firstSynonym = await page.evaluate(() => app.plugins.plugins.badi.session.text);
  await waitForFullRevision();
  await capture('selection-synonyms');
  await press('Tab');
  await page.waitForFunction(expected => app.workspace.activeLeaf.view.editor.getValue() === expected,
    synonymNote.replace(synonymSelection, firstSynonym));
  await press('Control+z');
  assert.equal(await text(), synonymNote, 'One native undo restores the synonym selection');
  await select(synonymNote, synonymSelection);
  await page.evaluate(() => app.commands.executeCommandById('badi:revise-synonyms'));
  await revision.waitFor({ state: 'visible', timeout: 47000 });
  const shownBeforeCycle = await page.evaluate(() => app.plugins.plugins.badi.session.text);
  await press('Control+Shift+Space');
  await page.waitForFunction(({ previous, local }) => {
    const controller = app.plugins.plugins.badi.session;
    return !controller?.pending && ((controller?.revision && controller.text && controller.text !== previous)
      || (local && controller.revisionReason !== null));
  }, { previous: shownBeforeCycle, local }, { timeout: 47000 });
  const nextSynonym = await page.evaluate(() => app.plugins.plugins.badi.session.text);
  const cycleReason = await page.evaluate(() => app.plugins.plugins.badi.session.revisionReason);
  assert.equal(await text(), synonymNote, 'Cycling synonyms does not edit the note');
  if (nextSynonym) {
    assert.notEqual(nextSynonym, shownBeforeCycle);
    await waitForFullRevision();
    await capture('selection-synonyms-next');
    await press('Escape');
  } else {
    assert.ok(local && ['no_change', 'invalid_output'].includes(cycleReason), 'Only a model abstention may clear the repeat');
  }
  await revision.waitFor({ state: 'hidden' });
  report.synonyms = { original: synonymSelection, first: firstSynonym, next: nextSynonym,
    cycle_reason: cycleReason, applied_and_undone: true, development_selected: local };

  const lowerNote = `${'Earlier line.\n'.repeat(32)}I has went.`;
  await select(lowerNote, 'I has went.');
  await page.evaluate(() => {
    const view = app.workspace.activeLeaf.view.editor.cm;
    view.scrollDOM.scrollTop = view.scrollDOM.scrollHeight;
  });
  await page.waitForFunction(() => {
    const view = app.workspace.activeLeaf.view.editor.cm;
    return view.coordsAtPos(view.state.selection.main.head) !== null;
  });
  await page.evaluate(() => {
    const view = app.workspace.activeLeaf.view.editor.cm;
    const caret = view.coordsAtPos(view.state.selection.main.head), pane = view.scrollDOM.getBoundingClientRect();
    view.scrollDOM.scrollTop += caret.bottom - pane.bottom + 24;
  });
  await press('Control+Shift+Space');
  await revision.waitFor({ state: 'visible', timeout: 47000 });
  await waitForFullRevision();
  report.revision_bottom = await revision.evaluate(element => {
    const view = app.workspace.activeLeaf.view.editor.cm;
    return { preview: element.getBoundingClientRect().toJSON(),
      anchor: view.coordsAtPos(view.state.selection.main.head), pane: view.scrollDOM.getBoundingClientRect().toJSON() };
  });
  assert.ok(report.revision_bottom.anchor.bottom > report.revision_bottom.pane.bottom - 100,
    'The lower selection really sits near the pane bottom');
  assert.ok(report.revision_bottom.preview.bottom <= report.revision_bottom.anchor.top,
    'The complete preview moves above a bottom selection');
  await capture('selection-bottom');
  await press('Escape');
  assert.equal(await text(), lowerNote);

  // A layout that cannot show the entire diff may still position a tooltip,
  // but Tab must not authorize that hidden or clipped replacement.
  await select(note, 'I has went.');
  await press('Control+Shift+Space');
  await revision.waitFor({ state: 'visible', timeout: 47000 });
  await page.evaluate(() => { document.querySelector('.badi-revision').style.height = '1px';
    document.querySelector('.badi-revision').style.overflow = 'hidden'; });
  await press('Tab');
  await revision.waitFor({ state: 'hidden' });
  assert.equal(await text(), note, 'A clipped marked preview cannot be accepted');
  report.samples.unshift({ language: 'en', prefix, suggestion });
  report.metrics_after_restart = (await control('status')).metrics;
  assert.equal(report.metrics_after_restart.provider_errors, 0);
  assert.equal(report.metrics_after_restart.commit_failures, 0);
  report.passed = ['automatic display', '800ms reading', 'Tab next word', 'Ctrl+Right all',
    'native undo', 'Escape', 'stale caret', 'same-note broker restart recovery',
    'marked selection revision', 'reverse Unicode selection', 'revision native undo', 'stale selection',
    'synonym native undo', 'bottom-pane revision preview', 'clipped revision refusal'];
  report.passed.push(nextSynonym ? 'repeated synonyms' : 'rejected synonym repeat leaves text unchanged');
  if (local) report.passed.push('unresolved typo suppression');
  report.success = true;
  console.log(JSON.stringify({ ...report, output }, null, 2));
} catch (error) {
  report.error = String(error);
  if (page && browser?.isConnected()) {
    report.page = await page.evaluate(() => ({ width: innerWidth, height: innerHeight,
      content: document.body.innerText.slice(0, 2500),
      editors: [...document.querySelectorAll('.cm-content')].map(element => ({
        rect: element.getBoundingClientRect().toJSON(), display: getComputedStyle(element).display,
      })),
      controllers: [...(app.plugins.plugins.badi?.views.values() ?? [])].map(view => ({
        text: view.text, pending: view.pending, blocked: view.blockedReason(),
        focused: view.view.hasFocus, decorations: view.decorations.size,
      })),
      escapeTrace: globalThis.badiEscapeTrace,
    })).catch(() => null);
    const session = await page.context().newCDPSession(page);
    const capture = await session.send('Page.captureScreenshot', { format: 'png' }).catch(() => null);
    if (capture) await writeFile(join(output, 'failure.png'), Buffer.from(capture.data, 'base64'));
    await session.detach();
    console.error(JSON.stringify({ output, page: report.page }, null, 2));
  }
  throw error;
} finally {
  await browser?.close().catch(() => undefined);
  if (app?.exitCode === null && app.pid) {
    // Signal the app only. A group signal delivers SIGTRAP to Electron's
    // crash helper and makes Omarchy open a crash-diagnosis agent.
    const exited = once(app, 'exit');
    try { process.kill(app.pid, 'SIGTERM'); } catch { /* already gone */ }
    const timer = setTimeout(() => { try { process.kill(app.pid, 'SIGKILL'); } catch { /* already gone */ } }, 5000);
    try { await exited; } finally { clearTimeout(timer); }
  }
  await stopBroker();
  report.finished_at = new Date().toISOString();
  await cp(join(env.XDG_RUNTIME_DIR, 'badi/debug-trace.jsonl'), join(output, 'debug-trace.jsonl'))
    .catch(error => { if (error.code !== 'ENOENT') throw error; });
  await writeFile(join(output, 'result.json'), JSON.stringify(report, null, 2));
  await writeFile(join(output, 'application.log'), appLog, { mode: 0o600 });
  await writeFile(join(output, 'broker.log'), brokerLog, { mode: 0o600 });
  await rm(root, { recursive: true, force: true });
  if (previousWindow && ownedAddress) {
    const unlocked = await unlockedDesktop().then(() => true, () => false);
    const activeWindow = await run('hyprctl', ['activewindow', '-j']).then(value => JSON.parse(value.stdout), () => null);
    if (unlocked && (!activeWindow?.address || activeWindow.address === ownedAddress)) {
      await focusWindow(previousWindow).catch(() => undefined);
    }
  }
}
