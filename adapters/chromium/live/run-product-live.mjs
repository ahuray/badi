#!/usr/bin/env node

import { execFile, spawn } from "node:child_process";
import { constants as fsConstants } from "node:fs";
import {
  access,
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { delimiter, dirname, isAbsolute, join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import {
  DILLINGER_OPTIONAL_HOST_PERMISSION,
  assertExactChromiumProductManifest,
} from "../scripts/product-manifest-policy.mjs";

const execFileAsync = promisify(execFile);
const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(packageRoot, "../..");
const distRoot = join(packageRoot, "dist-product");
const extensionId = "ckkiehcjbclcjckkkajohopoikeejkoa";
const extensionOrigin = `chrome-extension://${extensionId}/`;
const extensionWorkerUrl = `${extensionOrigin}product-service-worker.js`;
const dillingerUrl = "https://dillinger.io/";
const trigger = "thank you";
const completion = " for your time";
const brokerBinary = join(repositoryRoot, "target/debug/badi-broker");
const nativeHostBinary = join(repositoryRoot, "target/debug/badi-native-host");
const nativeManifestBinary = join(repositoryRoot, "target/debug/badi-native-manifest");
const cliBinary = join(repositoryRoot, "target/debug/badictl");
let receivedSignal = null;

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function parseArguments(values) {
  const parsed = {
    chromiumExecutable: null,
    headless: false,
    interactive: false,
    permissionTimeoutMs: 60_000,
  };
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    const next = values[index + 1];
    if (value === "--headless") {
      parsed.headless = true;
      continue;
    }
    if (value === "--interactive") {
      parsed.interactive = true;
      continue;
    }
    if (value === "--chromium-executable") {
      if (next === undefined || !isAbsolute(next)) {
        throw new Error("--chromium-executable requires an absolute path");
      }
      parsed.chromiumExecutable = next;
      index += 1;
      continue;
    }
    if (value === "--permission-timeout-ms") {
      if (next === undefined || !/^\d+$/u.test(next)) {
        throw new Error("--permission-timeout-ms requires a positive integer");
      }
      parsed.permissionTimeoutMs = Number(next);
      if (!Number.isSafeInteger(parsed.permissionTimeoutMs) || parsed.permissionTimeoutMs < 1) {
        throw new Error("--permission-timeout-ms is outside the supported range");
      }
      index += 1;
      continue;
    }
    throw new Error(`Unknown argument: ${String(value)}`);
  }
  if (parsed.headless && parsed.interactive) {
    throw new Error("--interactive requires headed Chromium");
  }
  return parsed;
}

async function resolveChromiumExecutable(configuredPath) {
  if (configuredPath !== null) {
    await access(configuredPath, fsConstants.X_OK);
    return configuredPath;
  }
  for (const directory of (process.env.PATH ?? "").split(delimiter)) {
    if (directory.length === 0) continue;
    const candidate = resolve(directory, "chromium");
    try {
      await access(candidate, fsConstants.X_OK);
      return candidate;
    } catch {
      // Continue searching PATH.
    }
  }
  throw new Error(
    "Chromium was not found on PATH; pass --chromium-executable ABSOLUTE_PATH",
  );
}

async function command(file, args, options = {}) {
  return execFileAsync(file, args, {
    cwd: options.cwd ?? repositoryRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
}

async function exists(path) {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

async function waitFor(label, predicate, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError = null;
  while (Date.now() < deadline) {
    if (receivedSignal !== null) throw new Error(`Interrupted by ${receivedSignal}`);
    try {
      const value = await predicate();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 40));
  }
  const suffix = lastError instanceof Error ? `: ${lastError.message}` : "";
  throw new Error(`Timed out waiting for ${label}${suffix}`);
}

async function waitForChildExit(child, timeoutMs) {
  if (child.exitCode !== null || child.signalCode !== null) return true;
  return Promise.race([
    new Promise((resolvePromise) => child.once("exit", () => resolvePromise(true))),
    new Promise((resolvePromise) => setTimeout(() => resolvePromise(false), timeoutMs)),
  ]);
}

async function stopChild(child) {
  if (child === null || child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  if (await waitForChildExit(child, 3_000)) return;
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  if (!(await waitForChildExit(child, 3_000))) {
    throw new Error(`Child process ${String(child.pid)} did not exit after SIGKILL`);
  }
}

async function pidsWithCommandFragment(fragment) {
  const pids = [];
  for (const name of await readdir("/proc")) {
    if (!/^\d+$/u.test(name) || Number(name) === process.pid) continue;
    try {
      const commandLine = (await readFile(join("/proc", name, "cmdline"), "utf8")).replaceAll(
        "\0",
        " ",
      );
      if (commandLine.includes(fragment)) pids.push(Number(name));
    } catch {
      // The process exited or is not readable.
    }
  }
  return pids;
}

async function waitForNoProcesses(fragment, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const pids = await pidsWithCommandFragment(fragment);
    if (pids.length === 0) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 40));
  }
  const pids = await pidsWithCommandFragment(fragment);
  check(pids.length === 0, `Processes still reference disposable state: ${pids.join(", ")}`);
}

async function holdForInteractiveDemo() {
  check(process.stdin.isTTY === true, "--interactive requires a terminal stdin");
  await new Promise((resolvePromise, reject) => {
    const cleanup = () => {
      clearInterval(timer);
      process.stdin.pause();
      process.stdin.removeListener("data", finish);
    };
    const timer = setInterval(() => {
      if (receivedSignal === null) return;
      cleanup();
      reject(new Error(`Interrupted by ${receivedSignal}`));
    }, 100);
    const finish = () => {
      cleanup();
      resolvePromise();
    };
    process.stdin.setEncoding("utf8");
    process.stdin.resume();
    process.stdin.once("data", finish);
  });
}

async function revokeDillingerAccess(worker, page) {
  const removed = await worker.evaluate(
    (permissionPattern) => chrome.permissions.remove({ origins: [permissionPattern] }),
    DILLINGER_OPTIONAL_HOST_PERMISSION,
  );
  check(removed === true, "Exact Dillinger permission removal failed");
  await waitFor("product registration removal", async () => {
    const registered = await worker.evaluate(() => chrome.scripting.getRegisteredContentScripts());
    return registered.length === 0;
  });
  await page.waitForFunction(
    () => {
      const preview = document.querySelector("[data-badi-dillinger-preview]");
      return !(preview instanceof HTMLElement) || preview.hidden;
    },
    undefined,
    { timeout: 5_000 },
  );
  return removed;
}

async function installNativeManifest({ profile, home, xdgConfig, manifest }) {
  const relative = join("NativeMessagingHosts", "io.github.ahuray.badi.json");
  for (const destination of [
    join(profile, relative),
    join(home, ".config/chromium", relative),
    join(xdgConfig, "chromium", relative),
  ]) {
    await mkdir(dirname(destination), { recursive: true, mode: 0o700 });
    await writeFile(destination, manifest, { encoding: "utf8", mode: 0o600 });
  }
}

async function grantDillingerPolicy(socketPath, env) {
  const settings = {
    schema: "badi.settings.v1",
    revision: 1,
    paused: false,
    subjects: [
      {
        identity: {
          kind: "browser_origin",
          adapter: "chromium",
          scheme: "https",
          host: "dillinger.io",
          port: 443,
        },
        permissions: {
          suggest: "allow",
          display: "allow",
          context_read: "allow",
          learn: "block",
          retention: { mode: "none" },
        },
      },
    ],
  };
  await command(
    cliBinary,
    [
      "--socket",
      socketPath,
      "settings",
      "replace",
      "--if-revision",
      "0",
      "--json",
      JSON.stringify(settings),
    ],
    { env },
  );
}

async function brokerStatus(socketPath, env) {
  const result = await command(cliBinary, ["--socket", socketPath, "status", "--json"], {
    env,
  });
  return JSON.parse(result.stdout);
}

function attachExtensionDiagnostics(target, errors) {
  target.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });
  if (typeof target.on === "function") {
    target.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  }
}

async function focusDillinger(worker, page, timeoutMs) {
  await page.bringToFront();
  const requestFocus = () => worker.evaluate(async (expectedUrl) => {
    const tabs = await chrome.tabs.query({});
    const tab = tabs.find((candidate) => candidate.url === expectedUrl);
    if (tab?.id === undefined || tab.windowId === undefined) return false;
    await chrome.tabs.update(tab.id, { active: true });
    await chrome.windows.update(tab.windowId, { focused: true });
    return (
      (await chrome.windows.get(tab.windowId)).focused === true &&
      tab.id === (await chrome.tabs.query({ active: true, lastFocusedWindow: true }))[0]?.id
    );
  }, dillingerUrl);
  if (!(await requestFocus())) {
    process.stdout.write(
      `${JSON.stringify({
        stage: "focus",
        action:
          "Click the exact Dillinger editor in the disposable Chromium window; the adapter intentionally rejects an unfocused browser.",
        target: dillingerUrl,
      })}\n`,
    );
    await waitFor(
      "the user-focused disposable Dillinger window",
      async () => (await requestFocus()) && (await page.evaluate(() => document.hasFocus())),
      timeoutMs,
    );
  }
  await page.waitForFunction(() => document.hasFocus(), undefined, { timeout: timeoutMs });
}

async function readMonacoState(page) {
  return page.evaluate(() => {
    const api = globalThis.monaco;
    const editor = api?.editor?.getEditors?.()[0];
    const model = api?.editor?.getModels?.()[0];
    if (editor === undefined || model === undefined || editor.getModel() !== model) {
      throw new Error("Dillinger Monaco editor is unavailable");
    }
    const position = editor.getPosition();
    const selection = editor.getSelection();
    if (position === null || selection === null) throw new Error("Dillinger caret is unavailable");
    const active = document.activeElement;
    return {
      value: model.getValue(),
      versionId: model.getVersionId(),
      offset: model.getOffsetAt(position),
      lineNumber: position.lineNumber,
      column: position.column,
      selection: {
        startLineNumber: selection.startLineNumber,
        startColumn: selection.startColumn,
        endLineNumber: selection.endLineNumber,
        endColumn: selection.endColumn,
      },
      editorFocus: editor.hasTextFocus(),
      documentFocus: document.hasFocus(),
      activeElement: active instanceof HTMLElement ? active.className : "",
      scrollTop: editor.getScrollTop(),
      scrollLeft: editor.getScrollLeft(),
    };
  });
}

function assertEditorInvariant(state, value, offset, label) {
  check(state.value === value, `${label} value mismatch`);
  check(state.offset === offset, `${label} caret offset mismatch`);
  check(
    state.selection.startLineNumber === state.selection.endLineNumber &&
      state.selection.startColumn === state.selection.endColumn,
    `${label} selection is not collapsed`,
  );
  check(state.editorFocus === true, `${label} lost Monaco text focus`);
  check(state.documentFocus === true, `${label} lost document focus`);
}

async function main() {
  const settings = parseArguments(process.argv.slice(2));
  const chromiumExecutable = await resolveChromiumExecutable(settings.chromiumExecutable);
  let tempRoot = null;
  let context = null;
  let broker = null;
  let result = null;
  let runError = null;
  let socketPath = null;
  let callerLog = null;
  try {
    await command("npm", ["run", "build:product"], { cwd: packageRoot });
    for (const path of [
      join(packageRoot, "manifest.product.json"),
      join(distRoot, "manifest.json"),
    ]) {
      assertExactChromiumProductManifest(JSON.parse(await readFile(path, "utf8")));
    }
    await command(
      "cargo",
      [
        "build",
        "--bin",
        "badi-broker",
        "--bin",
        "badi-native-host",
        "--bin",
        "badi-native-manifest",
        "--bin",
        "badictl",
      ],
      { cwd: repositoryRoot },
    );

    const { chromium } = await import("playwright");
    tempRoot = await mkdtemp(join(tmpdir(), "badi-product-live-"));
    const runtime = join(tempRoot, "runtime");
    const profile = join(tempRoot, "profile");
    const home = join(tempRoot, "home");
    const xdgConfig = join(tempRoot, "config");
    const xdgCache = join(tempRoot, "cache");
    const wrapper = join(tempRoot, "native-host-wrapper");
    callerLog = join(tempRoot, "native-host-caller.log");
    socketPath = join(runtime, "badi/broker.sock");
    await Promise.all(
      [runtime, profile, home, xdgConfig, xdgCache].map((path) =>
        mkdir(path, { recursive: true, mode: 0o700 }),
      ),
    );
    await writeFile(
      wrapper,
      '#!/bin/sh\numask 077\nprintf \'pid:%s\\narg:%s\\n\' "$$" "$1" >> "$BADI_PRODUCT_CALLER_LOG"\nexec "$BADI_PRODUCT_REAL_HOST" "$1" --socket "$BADI_PRODUCT_SOCKET" 2>> "$BADI_PRODUCT_CALLER_LOG"\n',
      { encoding: "utf8", mode: 0o700 },
    );
    await chmod(wrapper, 0o700);
    const nativeManifest = (await command(nativeManifestBinary, ["--host-path", wrapper])).stdout;
    const parsedNativeManifest = JSON.parse(nativeManifest);
    check(
      parsedNativeManifest.allowed_origins?.length === 1 &&
        parsedNativeManifest.allowed_origins[0] === extensionOrigin,
      "Native manifest did not bind the exact product extension origin",
    );
    await installNativeManifest({ profile, home, xdgConfig, manifest: nativeManifest });

    const isolatedEnv = {
      ...process.env,
      HOME: home,
      XDG_CONFIG_HOME: xdgConfig,
      XDG_CACHE_HOME: xdgCache,
      XDG_RUNTIME_DIR: runtime,
    };
    broker = spawn(brokerBinary, ["--socket", socketPath], {
      cwd: repositoryRoot,
      env: isolatedEnv,
      stdio: ["ignore", "ignore", "ignore"],
    });
    await waitFor("private broker socket", () => exists(socketPath));
    await grantDillingerPolicy(socketPath, isolatedEnv);

    const extensionErrors = [];
    context = await chromium.launchPersistentContext(profile, {
      executablePath: chromiumExecutable,
      headless: settings.headless,
      ignoreDefaultArgs: ["--disable-extensions"],
      env: {
        ...isolatedEnv,
        XDG_RUNTIME_DIR: process.env.XDG_RUNTIME_DIR ?? runtime,
        BADI_PRODUCT_CALLER_LOG: callerLog,
        BADI_PRODUCT_REAL_HOST: nativeHostBinary,
        BADI_PRODUCT_SOCKET: socketPath,
      },
      args: [
        ...(settings.headless ? ["--headless=new"] : []),
        `--disable-extensions-except=${distRoot}`,
        `--load-extension=${distRoot}`,
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-sync",
        "--no-pings",
      ],
      viewport: { width: 1280, height: 900 },
    });
    const existingWorker = context.serviceWorkers().find((item) => item.url() === extensionWorkerUrl);
    const worker =
      existingWorker ?? (await context.waitForEvent("serviceworker", { timeout: 10_000 }));
    check(worker.url() === extensionWorkerUrl, "Unexpected product extension service worker");
    attachExtensionDiagnostics(worker, extensionErrors);

    const initialContract = await worker.evaluate(async (permissionPattern) => ({
      commands: await chrome.commands.getAll(),
      permissions: await chrome.permissions.getAll(),
      registered: await chrome.scripting.getRegisteredContentScripts(),
      hasExactAccess: await chrome.permissions.contains({ origins: [permissionPattern] }),
    }), DILLINGER_OPTIONAL_HOST_PERMISSION);
    const acceptCommand = initialContract.commands.find(
      (command) => command.name === "accept-dillinger-suggestion",
    );
    check(acceptCommand?.shortcut === "Ctrl+Shift+Y", "Ctrl+Shift+Y is not assigned in Chromium");
    check(initialContract.hasExactAccess === false, "Disposable profile started pre-authorized");
    check(initialContract.permissions.origins?.length !== 1, "Unexpected initial host grant");
    check(initialContract.registered.length === 0, "Product content script started pre-registered");

    const accessPage = await context.newPage();
    attachExtensionDiagnostics(accessPage, extensionErrors);
    await accessPage.goto(`${extensionOrigin}product-access.html`, { waitUntil: "load" });
    await accessPage.waitForFunction(
      () => document.querySelector("#status")?.textContent === "Dillinger access is disabled.",
    );
    process.stdout.write(
      `${JSON.stringify({
        stage: "permission",
        action:
          "Approve Chromium's exact dillinger.io host prompt in the disposable browser window.",
        requested_origin: DILLINGER_OPTIONAL_HOST_PERMISSION,
      })}\n`,
    );
    await accessPage.locator("#enable").click();
    await waitFor(
      "the user-approved exact Dillinger host grant",
      () =>
        worker.evaluate(
          (permissionPattern) => chrome.permissions.contains({ origins: [permissionPattern] }),
          DILLINGER_OPTIONAL_HOST_PERMISSION,
        ),
      settings.permissionTimeoutMs,
    );
    await waitFor("the exact dynamic Dillinger content registration", async () => {
      const registered = await worker.evaluate(() => chrome.scripting.getRegisteredContentScripts());
      return registered.length === 1 ? registered : false;
    });
    const grantedContract = await worker.evaluate(async () => ({
      permissions: await chrome.permissions.getAll(),
      registered: await chrome.scripting.getRegisteredContentScripts(),
    }));
    check(
      JSON.stringify(grantedContract.permissions.origins) ===
        JSON.stringify([DILLINGER_OPTIONAL_HOST_PERMISSION]),
      "Chromium did not store only the exact declared optional origin",
    );
    check(
      grantedContract.registered.length === 1 &&
        grantedContract.registered[0]?.id === "badi-dillinger-product-v1" &&
        JSON.stringify(grantedContract.registered[0]?.matches) ===
          JSON.stringify(["https://dillinger.io:443/"]) &&
        grantedContract.registered[0]?.persistAcrossSessions === false,
      "Dynamic product content registration is not exact and ephemeral",
    );
    await accessPage.close();

    const dillingerPage = await context.newPage();
    await dillingerPage.goto(dillingerUrl, { waitUntil: "domcontentloaded", timeout: 30_000 });
    await dillingerPage.waitForFunction(
      () =>
        location.href === "https://dillinger.io/" &&
        globalThis.monaco?.editor?.getEditors?.().length === 1 &&
        globalThis.monaco?.editor?.getModels?.().length === 1,
      undefined,
      { timeout: 30_000 },
    );
    await focusDillinger(worker, dillingerPage, settings.permissionTimeoutMs);
    if (settings.interactive) {
      process.stdout.write(
        `${JSON.stringify({
          stage: "interactive",
          action:
            "In Dillinger, type the exact phrase_v1 trigger `thank you`, wait for Badi's preview, and press Ctrl+Shift+Y. Return here and press Enter to revoke access and remove all disposable state; Ctrl-C also cleans up.",
          target: dillingerUrl,
        })}\n`,
      );
      await holdForInteractiveDemo();
      const removed = await revokeDillingerAccess(worker, dillingerPage);
      check(extensionErrors.length === 0, `Extension errors: ${extensionErrors.join(" | ")}`);
      result = {
        record_version: 1,
        evidence_class: "interactive-disposable-demo-not-proof",
        release_evidence: false,
        target: dillingerUrl,
        browser: (await command(chromiumExecutable, ["--version"])).stdout.trim(),
        browser_mode: "headed",
        extension: {
          id: extensionId,
          worker: extensionWorkerUrl,
          optional_permission: DILLINGER_OPTIONAL_HOST_PERMISSION,
          runtime_url_gate: dillingerUrl,
          command: acceptCommand.shortcut,
        },
        native_bridge: {
          manifest_origin: parsedNativeManifest.allowed_origins[0],
          broker_socket: "isolated-temporary",
          provider: (await brokerStatus(socketPath, isolatedEnv)).provider,
        },
        interactive_session_held: true,
        revocation: {
          permission_removed: removed,
          dynamic_registration_removed: true,
          preview_cleared: true,
        },
      };
    } else {
      const statusBefore = await brokerStatus(socketPath, isolatedEnv);
      await dillingerPage.evaluate(() => {
        const editor = globalThis.monaco.editor.getEditors()[0];
        editor.focus();
        editor.setScrollPosition({ scrollTop: 0, scrollLeft: 0 });
      });
      await dillingerPage.keyboard.press("Control+A");
      await dillingerPage.keyboard.type(trigger, { delay: 20 });
      await dillingerPage.waitForFunction(
        (expected) => globalThis.monaco.editor.getModels()[0]?.getValue() === expected,
        trigger,
      );
      await dillingerPage.waitForFunction(
        () => {
          const preview = document.querySelector("[data-badi-dillinger-preview]");
          return preview instanceof HTMLElement && preview.hidden === false;
        },
        undefined,
        { timeout: 5_000 },
      );
      const before = await readMonacoState(dillingerPage);
      assertEditorInvariant(before, trigger, trigger.length, "before acceptance");
      await dillingerPage.keyboard.press("Control+Shift+Y");
      const acceptedValue = trigger + completion;
      await dillingerPage.waitForFunction(
        (expected) => globalThis.monaco.editor.getModels()[0]?.getValue() === expected,
        acceptedValue,
      );
      const accepted = await readMonacoState(dillingerPage);
      assertEditorInvariant(accepted, acceptedValue, acceptedValue.length, "after acceptance");
      check(accepted.scrollTop === before.scrollTop, "Acceptance changed Monaco vertical scroll");
      check(accepted.scrollLeft === before.scrollLeft, "Acceptance changed Monaco horizontal scroll");

      const removed = await revokeDillingerAccess(worker, dillingerPage);

      await dillingerPage.keyboard.press("Control+Z");
      await dillingerPage.waitForFunction(
        (expected) => globalThis.monaco.editor.getModels()[0]?.getValue() === expected,
        trigger,
      );
      const undone = await readMonacoState(dillingerPage);
      assertEditorInvariant(undone, trigger, trigger.length, "after native undo");
      check(undone.scrollTop === before.scrollTop, "Undo changed Monaco vertical scroll");
      check(undone.scrollLeft === before.scrollLeft, "Undo changed Monaco horizontal scroll");

      await dillingerPage.keyboard.press("Control+Shift+Z");
      await dillingerPage.waitForFunction(
        (expected) => globalThis.monaco.editor.getModels()[0]?.getValue() === expected,
        acceptedValue,
      );
      const redone = await readMonacoState(dillingerPage);
      assertEditorInvariant(redone, acceptedValue, acceptedValue.length, "after native redo");
      check(redone.scrollTop === before.scrollTop, "Redo changed Monaco vertical scroll");
      check(redone.scrollLeft === before.scrollLeft, "Redo changed Monaco horizontal scroll");

      await dillingerPage.keyboard.press("Control+Z");
      await dillingerPage.waitForFunction(
        (expected) => globalThis.monaco.editor.getModels()[0]?.getValue() === expected,
        trigger,
      );
      const statusAfter = await brokerStatus(socketPath, isolatedEnv);
      await waitFor("native host caller log", () => exists(callerLog));
      const callerLogBody = await readFile(callerLog, "utf8");
      check(
        callerLogBody.split("\n").includes(`arg:${extensionOrigin}`),
        "Chromium did not pass the exact extension origin to the native host",
      );
      check(
        statusAfter.metrics.provider_calls > statusBefore.metrics.provider_calls,
        "The real phrase_v1 provider was not reached",
      );
      check(extensionErrors.length === 0, `Extension errors: ${extensionErrors.join(" | ")}`);
      const acceptedExactExpectedOutput = accepted.value === acceptedValue;
      const undoRestoredExactFixture = undone.value === trigger;
      const redoRestoredExactAcceptance = redone.value === acceptedValue;

      result = {
        record_version: 1,
        evidence_class: "disposable-real-device-product-smoke",
        release_evidence: false,
        target: dillingerUrl,
        browser: (await command(chromiumExecutable, ["--version"])).stdout.trim(),
        browser_mode: settings.headless ? "headless" : "headed",
        extension: {
          id: extensionId,
          worker: extensionWorkerUrl,
          optional_permission: DILLINGER_OPTIONAL_HOST_PERMISSION,
          runtime_url_gate: dillingerUrl,
          command: acceptCommand.shortcut,
        },
        native_bridge: {
          manifest_origin: parsedNativeManifest.allowed_origins[0],
          broker_socket: "isolated-temporary",
          provider: statusAfter.provider,
          provider_calls_delta:
            statusAfter.metrics.provider_calls - statusBefore.metrics.provider_calls,
        },
        transaction: {
          phrase_case: "phrase_v1.thank-you",
          accepted_exact_expected_output: acceptedExactExpectedOutput,
          undo_restored_exact_fixture: undoRestoredExactFixture,
          redo_restored_exact_acceptance: redoRestoredExactAcceptance,
          focus_preserved: accepted.editorFocus && undone.editorFocus && redone.editorFocus,
          caret_offsets: [before.offset, accepted.offset, undone.offset, redone.offset],
          scroll_preserved:
            [accepted, undone, redone].every(
              (state) => state.scrollTop === before.scrollTop && state.scrollLeft === before.scrollLeft,
            ),
        },
        revocation: {
          permission_removed: removed,
          dynamic_registration_removed: true,
          preview_cleared: true,
        },
      };
    }
  } catch (error) {
    runError = error;
  } finally {
    const cleanupErrors = [];
    let trackedNativeHostPids = [];
    if (callerLog !== null && (await exists(callerLog))) {
      try {
        trackedNativeHostPids = (await readFile(callerLog, "utf8"))
          .split("\n")
          .filter((line) => /^pid:\d+$/u.test(line))
          .map((line) => Number(line.slice(4)));
      } catch (error) {
        cleanupErrors.push(error);
      }
    }
    try {
      if (context !== null) await context.close();
    } catch (error) {
      cleanupErrors.push(error);
    }
    try {
      await stopChild(broker);
    } catch (error) {
      cleanupErrors.push(error);
    }
    if (tempRoot !== null) {
      try {
        await waitForNoProcesses(tempRoot);
        const resolvedNativeHostPids = [];
        for (const pid of trackedNativeHostPids) {
          if (await exists(join("/proc", String(pid)))) resolvedNativeHostPids.push(pid);
        }
        check(
          resolvedNativeHostPids.length === 0,
          `Native-host processes remain: ${resolvedNativeHostPids.join(", ")}`,
        );
        await rm(tempRoot, { recursive: true, force: true });
        check(!(await exists(tempRoot)), "Disposable product tree was not removed");
        if (socketPath !== null) {
          check(!(await exists(socketPath)), "Disposable broker socket was not removed");
        }
        await waitForNoProcesses(tempRoot);
      } catch (error) {
        cleanupErrors.push(error);
      }
    }
    if (cleanupErrors.length > 0) {
      runError = new AggregateError(
        runError === null ? cleanupErrors : [runError, ...cleanupErrors],
        "Product live run or cleanup failed",
      );
    } else if (result !== null) {
      result.cleanup = {
        model_restored_to_trigger: settings.interactive ? null : true,
        disposable_profile_removed: true,
        broker_socket_removed: true,
        browser_and_native_processes_remaining: 0,
      };
    }
  }
  if (runError !== null) throw runError;
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

const onSigint = () => {
  receivedSignal = "SIGINT";
};
const onSigterm = () => {
  receivedSignal = "SIGTERM";
};
process.on("SIGINT", onSigint);
process.on("SIGTERM", onSigterm);
try {
  await main();
} catch (error) {
  if (receivedSignal === null) throw error;
  process.stderr.write(`Product live run cleaned up after ${receivedSignal}.\n`);
  process.exitCode = receivedSignal === "SIGINT" ? 130 : 143;
} finally {
  process.off("SIGINT", onSigint);
  process.off("SIGTERM", onSigterm);
}
