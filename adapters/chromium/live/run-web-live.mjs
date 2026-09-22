import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, mkdir, writeFile, readFile, cp, rm, lstat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { resolve, join } from 'node:path';
import { chromium } from 'playwright';

const ROOT = resolve(import.meta.dirname, '../../..');
const local = process.argv.includes('--local');
const multilingual = process.argv.includes('--multilingual');
if (multilingual && !local) throw new Error('--multilingual requires the actual local model');
const run = promisify(execFile);
const root = await mkdtemp('/tmp/badi-web-live-');
const env = { ...process.env, HOME: root, XDG_RUNTIME_DIR: join(root, 'runtime'),
  XDG_CONFIG_HOME: join(root, 'config'), XDG_DATA_HOME: join(root, 'data') };
const server = createServer((_req, response) => {
  response.writeHead(200, { 'Content-Type': 'text/html' });
  response.end('<!doctype html><html lang="en"><title>Writing integration test</title><style>body{font:20px system-ui;padding:40px}textarea,input{font:inherit;display:block;margin:16px 0;width:650px;padding:12px}textarea{height:150px}</style><h1>Badi writing test</h1><label for="note">Note</label><textarea id="note"></textarea><label for="line">Text</label><input id="line"><label for="secret">Password test</label><input id="secret" type="password"><p>Suggestions appear while typing. Tab accepts a word; Ctrl+Right accepts all. Escape dismisses.</p>');
});
server.listen(0, '127.0.0.1'); await once(server, 'listening');
const origin = `http://127.0.0.1:${server.address().port}`;
let broker, context;
let logs = '';
const endpoint = join(env.XDG_RUNTIME_DIR, 'badi/broker.sock');
const control = async (...args) => JSON.parse((await run(join(ROOT, 'target/release/badictl'), ['--socket', endpoint, ...args], { env })).stdout);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const report = { model: local ? 'local' : 'phrase', permission_test: 'pregranted localhost in disposable manifest; real permission prompt remains a headed check', samples: [], runtime: { browser: null, node: process.version },
  latency_boundary: 'Last DOM input event to suggestion host becoming visible in a rendered browser frame; includes debounce. Synthetic app samples, not a held-out quality benchmark.' };
async function startBroker() {
  broker = spawn(join(ROOT, 'target/release/badi-broker'), local
    ? ['--provider', 'local', '--model-directory', join(process.env.HOME, '.local/share/badi')]
    : ['--provider', 'phrase'], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  broker.stdout.on('data', data => { logs += data; }); broker.stderr.on('data', data => { logs += data; });
  for (let i = 0; ; i++) {
    assert.equal(broker.exitCode, null, logs);
    if (await lstat(endpoint).then(() => true, () => false)) break;
    if (i > 200) throw new Error(`Broker startup failed: ${logs}`);
    await delay(100);
  }
}
async function stopBroker() {
  if (broker?.exitCode !== null) return;
  const exit = once(broker, 'exit'); broker.kill('SIGTERM');
  const timeout = setTimeout(() => broker.kill('SIGKILL'), 5000);
  try { await exit; } finally { clearTimeout(timeout); }
}
try {
  for (const path of [env.XDG_RUNTIME_DIR, env.XDG_DATA_HOME, join(env.XDG_CONFIG_HOME, 'badi'), join(env.XDG_CONFIG_HOME, 'chromium/NativeMessagingHosts')]) await mkdir(path, { recursive: true, mode: 0o700 });
  await writeFile(join(env.XDG_CONFIG_HOME, 'badi/settings.json'), JSON.stringify({ schema: 'badi.settings.v2', revision: 1, paused: false, subjects: [{
    identity: { kind: 'browser_origin', adapter: 'chromium', scheme: 'http', host: '127.0.0.1', port: server.address().port },
    permissions: { context_read: 'allow', display: 'allow', suggest: 'allow', learn: 'block', retention: { mode: 'none' } },
  }] }), { mode: 0o600 });
  await startBroker();
  const wrapper = join(root, 'native-host');
  await writeFile(wrapper, `#!/bin/sh\nexec '${ROOT}/target/release/badi-native-host' "$@" --socket '${endpoint}'\n`, { mode: 0o700 });
  const manifest = (await run(join(ROOT, 'target/release/badi-native-manifest'), ['--host-path', wrapper])).stdout;
  await writeFile(join(env.XDG_CONFIG_HOME, 'chromium/NativeMessagingHosts/io.github.ahuray.badi.json'), manifest, { mode: 0o600 });
  await mkdir(join(root, 'profile/NativeMessagingHosts'), { recursive: true });
  await writeFile(join(root, 'profile/NativeMessagingHosts/io.github.ahuray.badi.json'), manifest);
  const extension = join(root, 'extension');
  await cp(join(ROOT, 'adapters/chromium/dist-web'), extension, { recursive: true });
  const extensionManifest = JSON.parse(await readFile(join(extension, 'manifest.json'), 'utf8'));
  extensionManifest.host_permissions = ['http://127.0.0.1/*'];
  await writeFile(join(extension, 'manifest.json'), JSON.stringify(extensionManifest));
  context = await chromium.launchPersistentContext(join(root, 'profile'), { executablePath: '/usr/bin/chromium', headless: true,
    ignoreDefaultArgs: ['--disable-extensions'], env, locale: 'en-US', viewport: { width: 1100, height: 800 }, args: [
      `--disable-extensions-except=${extension}`, `--load-extension=${extension}`, '--disable-background-networking', '--no-first-run',
    ] });
  context.setDefaultTimeout(5000);
  const errors = [];
  const worker = context.serviceWorkers()[0] ?? await context.waitForEvent('serviceworker');
  worker.on('console', message => { if (['error', 'warning'].includes(message.type())) errors.push(message.text()); });
  await worker.evaluate(() => { globalThis.badiTestEvents = []; chrome.runtime.onMessage.addListener((message, sender) => { if (globalThis.badiTestEvents.length < 20) globalThis.badiTestEvents.push({ kind: message.kind, sender }); }); });
  await worker.evaluate(async () => {
    for (let i = 0; i < 50; i++) {
      if ((await chrome.scripting.getRegisteredContentScripts()).length) return;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error('Site content script was not registered');
  });
  report.runtime.browser = context.browser()?.version() ?? await context.newCDPSession(context.pages()[0]).then(async session => {
    try { return (await session.send('Browser.getVersion')).product; } finally { await session.detach(); }
  });
  await context.addInitScript(() => {
    globalThis.badiLatency = { lastInput: null, visible: null };
    document.addEventListener('input', event => {
      if (event.target.id === 'note' || event.target.id === 'line') {
        globalThis.badiLatency = { lastInput: performance.now(), visible: null };
      }
    }, true);
    const frame = () => {
      const timing = globalThis.badiLatency;
      const host = document.querySelector('[data-badi-owned]');
      if (timing.lastInput !== null && timing.visible === null && host && !host.hidden && host.getBoundingClientRect().height > 0) {
        timing.visible = performance.now() - timing.lastInput;
      }
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  });
  const page = context.pages()[0];
  page.on('pageerror', error => errors.push(String(error)));
  page.on('console', message => { if (['error', 'warning'].includes(message.type())) errors.push(message.text()); });
  await page.goto(origin); await page.bringToFront();
  const field = page.locator('#note');
  const ghost = page.locator('[data-badi-owned]');
  const prefix = local ? 'Please find attached the' : 'thank you';
  await field.click(); await field.pressSequentially(prefix, { delay: 25 });
  try { await ghost.waitFor({ state: 'visible', timeout: 5000 }); }
  catch (error) { throw new Error(`${error}\n${JSON.stringify(await control('status'))}\n${errors.join('\n')}\n${JSON.stringify(await worker.evaluate(async () => ({ events: globalThis.badiTestEvents, tabs: await chrome.tabs.query({}), windows: await chrome.windows.getAll() })))}`); }
  const cdp = await context.newCDPSession(page);
  const tree = await cdp.send('DOM.getDocument', { depth: -1, pierce: true });
  function findSuggestion(node) {
    const attributes = node.attributes ?? [];
    if (attributes.some((value, index) => value === 'class' && attributes[index + 1] === 'suggestion'))
      return (node.children ?? []).map(child => child.nodeValue).join('');
    for (const child of [...(node.children ?? []), ...(node.shadowRoots ?? [])]) {
      const found = findSuggestion(child); if (found !== null) return found;
    }
    return null;
  }
  const suggestion = findSuggestion(tree.root);
  assert.equal(typeof suggestion, 'string');
  assert.ok(suggestion.trim().length > 0, 'The visible prediction must contain text');
  report.samples.push({ language: 'en', prefix, suggestion, visible_latency_ms: await page.evaluate(() => globalThis.badiLatency.visible) });
  await delay(800);
  assert.equal(await ghost.isVisible(), true, `Suggestion must survive a human reading pause: ${JSON.stringify(await control('status'))}`);
  await mkdir(join(ROOT, 'output/playwright'), { recursive: true });
  const capture = await cdp.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(join(ROOT, 'output/playwright/browser-suggestion.png'), Buffer.from(capture.data, 'base64'));
  await page.keyboard.press('Tab');
  try { await page.waitForFunction(prefix => document.querySelector('#note').value !== prefix, prefix, { timeout: 3000 }); }
  catch (error) { throw new Error(`${error}\n${JSON.stringify(await control('status'))}\n${errors.join('\n')}\n${JSON.stringify(await worker.evaluate(() => globalThis.badiTestEvents.map(e => e.kind)))}`); }
  const accepted = await field.inputValue();
  const firstWord = /^\s*\S+/u.exec(suggestion)?.[0];
  assert.ok(firstWord);
  assert.equal(accepted, prefix + firstWord, 'Tab accepts only the first suggested word');
  await page.keyboard.press('Control+z');
  assert.equal(await field.inputValue(), prefix);
  await page.keyboard.press('Tab'); await ghost.waitFor({ state: 'visible' });
  const fullSuggestion = findSuggestion((await cdp.send('DOM.getDocument', { depth: -1, pierce: true })).root);
  await page.keyboard.press('Control+ArrowRight');
  await page.waitForFunction(expected => document.querySelector('#note').value === expected, prefix + fullSuggestion);
  await page.keyboard.press('Control+z');
  assert.equal(await field.inputValue(), prefix, 'Full acceptance is one native undo step');
  await page.keyboard.press('Tab'); await ghost.waitFor({ state: 'visible' });
  await page.keyboard.press('Escape'); assert.equal(await ghost.isVisible(), false);
  assert.equal(await field.inputValue(), prefix);
  await page.keyboard.press('Tab'); await ghost.waitFor({ state: 'visible' });
  await page.keyboard.press('ArrowLeft'); await ghost.waitFor({ state: 'hidden', timeout: 500 });
  assert.equal(await field.inputValue(), prefix);
  const beforePassword = (await control('status')).metrics.provider_calls;
  await page.locator('#secret').fill('private-test-value'); await delay(800);
  assert.equal((await control('status')).metrics.provider_calls, beforePassword);
  assert.equal(await ghost.isVisible(), false);
  await page.locator('#line').click(); await page.locator('#line').pressSequentially(prefix, { delay: 25 });
  await ghost.waitFor({ state: 'visible' });
  await control('pause', 'on');
  await ghost.waitFor({ state: 'hidden' });
  assert.equal(await page.locator('#line').inputValue(), prefix);
  await control('pause', 'off');
  await ghost.waitFor({ state: 'visible', timeout: 5000 });
  await page.keyboard.press('Tab');
  await page.waitForFunction(prefix => document.querySelector('#line').value !== prefix, prefix);
  await field.fill('');
  await field.pressSequentially(prefix, { delay: 25 });
  await ghost.waitFor({ state: 'visible' });
  report.before_restart_metrics = (await control('status')).metrics;
  const recoveryErrorStart = errors.length;
  await stopBroker();
  await ghost.waitFor({ state: 'hidden', timeout: 1000 });
  await startBroker();
  try { await ghost.waitFor({ state: 'visible', timeout: 15000 }); }
  catch (error) { throw new Error(`Same-page recovery: ${error}; status=${JSON.stringify(await control('status'))}; diagnostics=${JSON.stringify(errors)}`); }
  const reconnectDiagnostics = errors.splice(recoveryErrorStart);
  assert.ok(reconnectDiagnostics.length <= 10 && reconnectDiagnostics.every(message =>
    message === 'Badi bootstrap failed Error: Native host has exited.'),
  `Only bounded connection failures are expected during the intentional broker outage: ${JSON.stringify(reconnectDiagnostics)}`);
  report.restart = { same_page_recovered: true, expected_connection_failures: reconnectDiagnostics };
  const recovered = findSuggestion((await cdp.send('DOM.getDocument', { depth: -1, pierce: true })).root);
  await page.keyboard.press('Control+ArrowRight');
  await page.waitForFunction(expected => document.querySelector('#note').value === expected, prefix + recovered);
  await page.keyboard.press('Control+z');
  assert.equal(await field.inputValue(), prefix, 'The same page recovers after broker restart and retains native undo');
  if (process.argv.includes('--popup')) {
    await page.locator('h1').click();
    await worker.evaluate(() => chrome.action.openPopup());
    await delay(300);
    const targets = await cdp.send('Target.getTargets');
    const popup = targets.targetInfos.find(target => target.url.endsWith('/web-access.html'));
    assert.ok(popup, 'The browser must open the real extension popup');
    const { sessionId } = await cdp.send('Target.attachToTarget', { targetId: popup.targetId, flatten: false });
    let sequence = 0;
    const evaluatePopup = expression => new Promise((resolve, reject) => {
      const id = ++sequence;
      const timeout = setTimeout(() => finish(new Error('Popup evaluation timed out')), 5000);
      const receive = event => {
        if (event.sessionId !== sessionId) return;
        const message = JSON.parse(event.message);
        if (message.id !== id) return;
        finish(message.error || message.result?.exceptionDetails
          ? new Error(JSON.stringify(message)) : null, message.result?.result?.value);
      };
      function finish(error, value) {
        clearTimeout(timeout); cdp.off('Target.receivedMessageFromTarget', receive);
        if (error) reject(error); else resolve(value);
      }
      cdp.on('Target.receivedMessageFromTarget', receive);
      void cdp.send('Target.sendMessageToTarget', { sessionId, message: JSON.stringify({ id,
        method: 'Runtime.evaluate', params: { expression, awaitPromise: true, returnByValue: true } }) }).catch(finish);
    });
    const ready = `new Promise((resolve, reject) => {
      const deadline = Date.now() + 4000;
      const tick = () => {
        const status = document.querySelector('#status').textContent;
        if (!document.querySelector('#retry').disabled && status.startsWith('Connected.')) resolve(status);
        else if (Date.now() > deadline) reject(new Error(status)); else setTimeout(tick, 30);
      }; tick();
    })`;
    await evaluatePopup(ready);
    await evaluatePopup("document.querySelector('#retry').click()");
    await evaluatePopup(ready);
    await cdp.send('Target.closeTarget', { targetId: popup.targetId });
    report.popup = 'Real extension popup connected and explicit retry completed';
    await page.bringToFront();
  }
  const languages = multilingual ? [
    ['en', 'Please review the docum'],
    ['de', 'Vielen Dank für Ihre'],
    ['fa', 'از همکاری شما بسیار'],
  ] : [['en', prefix]];
  for (const [language, samplePrefix] of languages) {
    for (let repetition = 0; repetition < 5; repetition++) {
      await field.evaluate((element, language) => {
        element.lang = language;
        element.dir = language === 'fa' ? 'rtl' : 'ltr';
      }, language);
      await field.fill('');
      await field.pressSequentially(samplePrefix, { delay: 25 });
      await ghost.waitFor({ state: 'visible' });
      await page.waitForFunction(() => globalThis.badiLatency.visible !== null);
      const text = findSuggestion((await cdp.send('DOM.getDocument', { depth: -1, pierce: true })).root);
      report.samples.push({ language, prefix: samplePrefix, suggestion: text,
        visible_latency_ms: Math.round(await page.evaluate(() => globalThis.badiLatency.visible)) });
      if (repetition === 0) {
        const frame = await cdp.send('Page.captureScreenshot', { format: 'png' });
        await writeFile(join(ROOT, `output/playwright/browser-${language}.png`), Buffer.from(frame.data, 'base64'));
      }
      await page.keyboard.press('Control+ArrowRight');
      try {
        await page.waitForFunction(expected => document.querySelector('#note').value === expected, samplePrefix + text);
      } catch (error) {
        throw new Error(`${language} acceptance failed: ${error}; expected=${JSON.stringify(samplePrefix + text)}; actual=${JSON.stringify(await field.inputValue())}; metrics=${JSON.stringify((await control('status')).metrics)}`);
      }
      await page.keyboard.press('Control+z');
      assert.equal(await field.inputValue(), samplePrefix, `${language}: native undo restores the prefix`);
    }
  }
  const latency = report.samples.map(sample => sample.visible_latency_ms).filter(Number.isFinite).sort((a, b) => a - b);
  report.visible_latency_ms = { samples: latency.length,
    p50: latency[Math.ceil(latency.length * .5) - 1], p95: latency[Math.ceil(latency.length * .95) - 1] };
  report.after_restart_metrics = (await control('status')).metrics;
  report.metrics = Object.fromEntries(Object.entries(report.after_restart_metrics)
    .map(([key, value]) => [key, value + report.before_restart_metrics[key]]));
  assert.equal(report.metrics.provider_errors, 0);
  assert.deepEqual(errors, []);
  report.passed = ['textarea prediction', 'input prediction', '800ms reading', 'Tab next-word accept', 'Ctrl+Right full accept', 'undo', 'Escape', 'caret invalidation', 'password excluded before acquisition', 'pause revocation', 'resume', 'same-page broker restart recovery'];
  report.success = true;
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  report.success = false;
  report.error = String(error);
  throw error;
} finally {
  await context?.close();
  await stopBroker();
  server.close();
  await mkdir(join(ROOT, 'output/playwright'), { recursive: true });
  await writeFile(join(ROOT, 'output/playwright/browser-live-result.json'), JSON.stringify(report, null, 2));
  await rm(root, { recursive: true, force: true });
}
