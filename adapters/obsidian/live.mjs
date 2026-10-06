import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, mkdir, writeFile, readFile, cp, rm, lstat } from 'node:fs/promises';
import { once } from 'node:events';
import { resolve, join } from 'node:path';
import { chromium } from 'playwright';

// Opt-in: the actual Obsidian application, a disposable vault/profile and broker.
// CDP drives only this process. No existing vault, plugin or service is changed.
const ROOT = resolve(import.meta.dirname, '../..');
const local = process.argv.includes('--local');
const headless = process.argv.includes('--headless');
const run = promisify(execFile);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let previousWindow;
if (!headless) {
  const lock = JSON.parse((await run('omarchy-shell', ['lock', 'status'])).stdout);
  assert.ok(['locked', 'requested', 'pending', 'sessionLocked', 'secure'].every(key => lock[key] === false),
    'Run the visible Obsidian trial after normal desktop unlock');
  previousWindow = JSON.parse((await run('hyprctl', ['activewindow', '-j'])).stdout).address;
}
const root = await mkdtemp('/tmp/badi-obsidian-live-');
await mkdir(join(ROOT, 'output/playwright'), { recursive: true });
const output = await mkdtemp(join(ROOT, 'output/playwright/obsidian-'));
const profile = join(root, 'profile');
const vault = join(root, 'Badi writing trial');
const env = { ...process.env, HOME: root, XDG_RUNTIME_DIR: join(root, 'runtime'),
  XDG_CONFIG_HOME: join(root, 'config'), XDG_DATA_HOME: join(root, 'data'),
  XDG_CACHE_HOME: join(root, 'cache'),
  WAYLAND_DISPLAY: resolve(process.env.XDG_RUNTIME_DIR, process.env.WAYLAND_DISPLAY) };
if (headless) { delete env.WAYLAND_DISPLAY; delete env.DISPLAY; }
const endpoint = join(env.XDG_RUNTIME_DIR, 'badi/broker.sock');
const control = async (...args) => JSON.parse((await run(join(ROOT, 'target/release/badictl'),
  ['--socket', endpoint, ...args], { env, timeout: 5000 })).stdout);
const report = { provider: local ? 'local_model' : 'phrase_fixture', success: false, samples: [],
  boundary: `Actual Obsidian/Electron with ${headless ? 'headless Ozone' : 'a Wayland window'}, isolated vault/profile and real broker; synthetic text. This lane does not measure writing usefulness or visible latency.` };
let broker, app, browser, page;
let brokerLog = '', appLog = '';
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
    schema: 'badi.settings.v2', revision: 1, paused: false, subjects: [{
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
  await page.bringToFront();
  // Trust only this newly created vault containing the plugin built above.
  await page.getByRole('button', { name: 'Trust author and enable plugins', exact: true }).click();
  const editor = page.locator('.workspace-leaf.mod-active .cm-content');
  await editor.click();
  await page.waitForFunction(() => app.plugins.plugins.badi?.views.size > 0);
  const ghost = page.locator('.badi-inline');
  const text = () => page.evaluate(() => app.workspace.activeLeaf.view.editor.getValue());
  const capture = async name => {
    const image = await cdp.send('Page.captureScreenshot', { format: 'png' });
    await writeFile(join(output, `${name}.png`), Buffer.from(image.data, 'base64'));
  };
  const setPrefix = async prefix => {
    await page.evaluate(() => app.workspace.activeLeaf.view.editor.setValue(''));
    await editor.click();
    await page.keyboard.insertText(prefix);
  };
  const prefix = local ? 'Please find attached the' : 'thank you';
  await setPrefix(prefix);
  await ghost.waitFor({ state: 'visible' });
  const suggestion = await ghost.textContent();
  assert.ok(suggestion?.trim());
  await capture('english');
  await delay(800);
  assert.equal(await ghost.isVisible(), true, 'The preview survives a reading pause');
  await page.keyboard.press('Tab');
  const firstWord = /^\s*\S+/u.exec(suggestion)[0];
  await page.waitForFunction(expected => app.workspace.activeLeaf.view.editor.getValue() === expected, prefix + firstWord);
  await page.keyboard.press('Control+z');
  assert.equal(await text(), prefix, 'One native undo removes exactly the accepted word');
  await page.keyboard.press('Tab');
  await ghost.waitFor({ state: 'visible' });
  const all = await ghost.textContent();
  await page.keyboard.press('Control+ArrowRight');
  await page.waitForFunction(expected => app.workspace.activeLeaf.view.editor.getValue() === expected, prefix + all);
  await page.keyboard.press('Control+z');
  assert.equal(await text(), prefix, 'Full acceptance is one native undo step');
  await page.keyboard.press('Tab'); await ghost.waitFor({ state: 'visible' });
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
  await page.keyboard.press('Escape');
  await ghost.waitFor({ state: 'hidden', timeout: 500 });
  report.escape_event = await page.evaluate(() => badiEscapeTrace.find(event => event.where === 'badi-observer'));
  assert.equal(report.escape_event?.trusted, true, 'The editor observer received the real Escape event');
  await delay(600);
  assert.equal(await ghost.count(), 0, 'Dismissal stays quiet until new input or explicit invocation');
  assert.equal(await text(), prefix);
  await page.keyboard.press('Tab'); await ghost.waitFor({ state: 'visible' });
  await page.keyboard.press('ArrowLeft');
  await ghost.waitFor({ state: 'hidden', timeout: 500 });
  await setPrefix(prefix); await ghost.waitFor({ state: 'visible' });
  await stopBroker();
  await ghost.waitFor({ state: 'hidden' });
  await startBroker();
  await ghost.waitFor({ state: 'visible', timeout: 15000 });
  const afterRestart = await ghost.textContent();
  await page.keyboard.press('Control+ArrowRight');
  await page.waitForFunction(expected => app.workspace.activeLeaf.view.editor.getValue() === expected, prefix + afterRestart);
  await page.keyboard.press('Control+z');
  assert.equal(await text(), prefix, 'The open note recovers after broker restart');
  if (local) {
    for (const [language, before] of [['de', 'Vielen Dank für Ihre'], ['fa', 'از همکاری شما بسیار']]) {
      await page.evaluate(language => app.commands.executeCommandById(`badi:language-${language}`), language);
      await setPrefix(before); await ghost.waitFor({ state: 'visible' });
      const continuation = await ghost.textContent();
      await capture(language);
      await page.keyboard.press('Control+ArrowRight');
      await page.waitForFunction(expected => app.workspace.activeLeaf.view.editor.getValue() === expected, before + continuation);
      await page.keyboard.press('Control+z');
      assert.equal(await text(), before);
      report.samples.push({ language, prefix: before, suggestion: continuation });
    }
  }
  report.samples.unshift({ language: 'en', prefix, suggestion });
  report.metrics_after_restart = (await control('status')).metrics;
  assert.equal(report.metrics_after_restart.provider_errors, 0);
  assert.equal(report.metrics_after_restart.commit_failures, 0);
  report.passed = ['automatic display', '800ms reading', 'Tab next word', 'Ctrl+Right all',
    'native undo', 'Escape', 'stale caret', 'same-note broker restart recovery'];
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
  await browser?.close();
  if (app?.exitCode === null) {
    const exited = once(app, 'exit');
    process.kill(-app.pid, 'SIGTERM');
    const timer = setTimeout(() => { try { process.kill(-app.pid, 'SIGKILL'); } catch {} }, 5000);
    try { await exited; } finally { clearTimeout(timer); }
  }
  await stopBroker();
  await writeFile(join(output, 'result.json'), JSON.stringify(report, null, 2));
  await writeFile(join(output, 'application.log'), appLog, { mode: 0o600 });
  await writeFile(join(output, 'broker.log'), brokerLog, { mode: 0o600 });
  await rm(root, { recursive: true, force: true });
  if (previousWindow) await run('hyprctl', ['dispatch', 'focuswindow', `address:${previousWindow}`]).catch(() => undefined);
}
