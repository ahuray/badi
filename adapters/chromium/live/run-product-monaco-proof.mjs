#!/usr/bin/env node

import { execFile } from "node:child_process";
import { constants as fsConstants } from "node:fs";
import { access, mkdir, mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, dirname, isAbsolute, join, resolve } from "node:path";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import {
  DILLINGER_OPTIONAL_HOST_PERMISSION,
  assertExactChromiumProductManifest,
} from "../scripts/product-manifest-policy.mjs";

const execFileAsync = promisify(execFile);
const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const distRoot = join(packageRoot, "dist-product");
const extensionId = "ckkiehcjbclcjckkkajohopoikeejkoa";
const extensionOrigin = `chrome-extension://${extensionId}/`;
const workerUrl = `${extensionOrigin}product-service-worker.js`;
const dillingerUrl = "https://dillinger.io/";
const trigger = "thank you";
const completion = " for your time";

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function parseArguments(values) {
  const parsed = { chromiumExecutable: null, headed: false };
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (value === "--headed") {
      parsed.headed = true;
      continue;
    }
    if (value === "--chromium-executable") {
      const next = values[index + 1];
      if (next === undefined || !isAbsolute(next)) {
        throw new Error("--chromium-executable requires an absolute path");
      }
      parsed.chromiumExecutable = next;
      index += 1;
      continue;
    }
    throw new Error(`Unknown argument: ${String(value)}`);
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

async function command(file, args, cwd = packageRoot) {
  return execFileAsync(file, args, {
    cwd,
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
  check(pids.length === 0, `Chromium processes still reference the disposable profile: ${pids.join(", ")}`);
}

function attachExtensionDiagnostics(target, errors) {
  target.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });
  target.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
}

async function compileProductModule(entryPoint, globalName, globalKey) {
  const output = await build({
    absWorkingDir: packageRoot,
    entryPoints: [entryPoint],
    bundle: true,
    write: false,
    charset: "utf8",
    format: "iife",
    globalName,
    platform: "browser",
    target: ["chrome132"],
    legalComments: "none",
    logLevel: "silent",
    minify: false,
    sourcemap: false,
    footer: {
      js: `globalThis.${globalKey} = ${globalName};`,
    },
  });
  const file = output.outputFiles[0];
  check(file !== undefined, `esbuild did not emit ${entryPoint}`);
  return file.text;
}

async function structuralMonacoState(page, expectedValue) {
  return page.evaluate((expected) => {
    const editor = globalThis.monaco.editor.getEditors()[0];
    const model = globalThis.monaco.editor.getModels()[0];
    const position = editor.getPosition();
    const selection = editor.getSelection();
    if (position === null || selection === null) throw new Error("Dillinger caret unavailable");
    return {
      exactExpectedValue: model.getValue() === expected,
      valueLength: model.getValueLength(),
      offset: model.getOffsetAt(position),
      collapsed:
        selection.startLineNumber === selection.endLineNumber &&
        selection.startColumn === selection.endColumn,
      editorFocus: editor.hasTextFocus(),
      documentFocus: document.hasFocus(),
      visibility: document.visibilityState,
      scrollTop: editor.getScrollTop(),
      scrollLeft: editor.getScrollLeft(),
    };
  }, expectedValue);
}

function assertStructuralState(state, expectedLength, expectedOffset, label) {
  check(state.exactExpectedValue === true, `${label} model content mismatch`);
  check(state.valueLength === expectedLength, `${label} model length mismatch`);
  check(state.offset === expectedOffset, `${label} caret mismatch`);
  check(state.collapsed === true, `${label} selection is not collapsed`);
  check(state.editorFocus === true, `${label} lost Monaco text focus`);
  check(state.documentFocus === true, `${label} lost document focus`);
  check(state.visibility === "visible", `${label} document is not visible`);
}

async function main() {
  const settings = parseArguments(process.argv.slice(2));
  const chromiumExecutable = await resolveChromiumExecutable(settings.chromiumExecutable);
  let context = null;
  let tempRoot = null;
  let result = null;
  let runError = null;
  try {
    await command("npm", ["run", "build:product"]);
    for (const path of [
      join(packageRoot, "manifest.product.json"),
      join(distRoot, "manifest.json"),
    ]) {
      assertExactChromiumProductManifest(JSON.parse(await readFile(path, "utf8")));
    }
    const [mainWorldSource, monacoViewSource] = await Promise.all([
      compileProductModule(
        "src/product/monaco-main-world.ts",
        "BadiProductMainWorld",
        "__BADI_PRODUCT_MAIN_WORLD_PROOF__",
      ),
      compileProductModule(
        "src/product/monaco-view.ts",
        "BadiProductMonacoView",
        "__BADI_PRODUCT_MONACO_VIEW_PROOF__",
      ),
    ]);
    const { chromium } = await import("playwright");
    tempRoot = await mkdtemp(join(tmpdir(), "badi-product-monaco-proof-"));
    const profile = join(tempRoot, "profile");
    const home = join(tempRoot, "home");
    const xdgConfig = join(tempRoot, "config");
    const xdgCache = join(tempRoot, "cache");
    const runtime = join(tempRoot, "runtime");
    await Promise.all(
      [profile, home, xdgConfig, xdgCache, runtime].map((path) =>
        mkdir(path, { recursive: true, mode: 0o700 }),
      ),
    );

    const extensionErrors = [];
    context = await chromium.launchPersistentContext(profile, {
      executablePath: chromiumExecutable,
      headless: !settings.headed,
      ignoreDefaultArgs: ["--disable-extensions"],
      env: {
        ...process.env,
        HOME: home,
        XDG_CONFIG_HOME: xdgConfig,
        XDG_CACHE_HOME: xdgCache,
        XDG_RUNTIME_DIR:
          settings.headed && process.env.XDG_RUNTIME_DIR !== undefined
            ? process.env.XDG_RUNTIME_DIR
            : runtime,
      },
      args: [
        ...(settings.headed ? [] : ["--headless=new"]),
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
    const existingWorker = context.serviceWorkers().find((worker) => worker.url() === workerUrl);
    const worker =
      existingWorker ?? (await context.waitForEvent("serviceworker", { timeout: 10_000 }));
    check(worker.url() === workerUrl, "Unexpected product extension service worker");
    attachExtensionDiagnostics(worker, extensionErrors);

    const contract = await worker.evaluate(async (permissionPattern) => ({
      command: (await chrome.commands.getAll()).find(
        (entry) => entry.name === "accept-dillinger-suggestion",
      ),
      permissions: await chrome.permissions.getAll(),
      registered: await chrome.scripting.getRegisteredContentScripts(),
      hasExactAccess: await chrome.permissions.contains({ origins: [permissionPattern] }),
    }), DILLINGER_OPTIONAL_HOST_PERMISSION);
    check(contract.command?.shortcut === "Ctrl+Shift+Y", "Ctrl+Shift+Y is not assigned");
    check(contract.hasExactAccess === false, "Disposable probe profile started authorized");
    check(contract.permissions.origins?.length !== 1, "Probe obtained an unexpected host grant");
    check(contract.registered.length === 0, "Probe registered a content script without a grant");

    const popup = await context.newPage();
    attachExtensionDiagnostics(popup, extensionErrors);
    await popup.goto(`${extensionOrigin}product-access.html`, { waitUntil: "load" });
    await popup.waitForFunction(
      () => document.querySelector("#status")?.textContent === "Dillinger access is disabled.",
    );
    const popupAssets = await popup.evaluate(async () => {
      const responses = await Promise.all(
        ["product-access.js", "product-content-script.js", "product-service-worker.js"].map(
          async (name) => {
            const response = await fetch(chrome.runtime.getURL(name));
            return { name, ok: response.ok, bytes: (await response.arrayBuffer()).byteLength };
          },
        ),
      );
      return responses;
    });
    check(
      popupAssets.every((asset) => asset.ok && asset.bytes > 0),
      "A required product build asset is unavailable",
    );
    await popup.close();

    const page = await context.newPage();
    await page.goto(dillingerUrl, { waitUntil: "domcontentloaded", timeout: 30_000 });
    await page.waitForFunction(
      () =>
        location.href === "https://dillinger.io/" &&
        globalThis.monaco?.editor?.getEditors?.().length === 1 &&
        globalThis.monaco?.editor?.getModels?.().length === 1,
      undefined,
      { timeout: 30_000 },
    );
    await page.bringToFront();
    await page.evaluate(() => {
      const editor = globalThis.monaco.editor.getEditors()[0];
      const model = globalThis.monaco.editor.getModels()[0];
      globalThis.__BADI_PRODUCT_MONACO_PROOF_RESTORE__ = {
        value: model.getValue(),
        selection: editor.getSelection(),
        scrollTop: editor.getScrollTop(),
        scrollLeft: editor.getScrollLeft(),
        focused: editor.hasTextFocus(),
      };
      model.setValue("thank you");
      editor.setPosition({ lineNumber: 1, column: 10 });
      editor.setScrollPosition({ scrollTop: 0, scrollLeft: 0 });
      editor.focus();
    });
    await page.waitForFunction(() => document.hasFocus() && document.visibilityState === "visible");
    await page.evaluate(monacoViewSource);
    const previewProof = await page.evaluate(() => {
      const Preview = globalThis.__BADI_PRODUCT_MONACO_VIEW_PROOF__.MonacoGhostView;
      const preview = new Preview(document);
      const geometry = {
        left: innerWidth - 2,
        top: innerHeight - 40,
        height: 16,
      };
      preview.show("fixed product proof", geometry);
      const host = document.querySelector("[data-badi-dillinger-preview]");
      if (!(host instanceof HTMLElement)) throw new Error("Product preview host unavailable");
      const box = host.getBoundingClientRect();
      const initiallyVisible = preview.visible;
      const fullyInViewport =
        box.width > 0 &&
        box.height > 0 &&
        box.left >= 0 &&
        box.top >= 0 &&
        box.right <= innerWidth &&
        box.bottom <= innerHeight;
      const blocker = document.createElement("div");
      blocker.style.cssText = [
        "position:fixed",
        `left:${String(box.left)}px`,
        `top:${String(box.top)}px`,
        `width:${String(box.width)}px`,
        `height:${String(box.height)}px`,
        "z-index:2147483647",
        "pointer-events:auto",
      ].join(";");
      document.documentElement.append(blocker);
      const occlusionRejected = preview.visible === false;
      blocker.remove();
      preview.dispose();
      return {
        initiallyVisible,
        fullyInViewport,
        rightEdgeClamped: box.right <= innerWidth - 7,
        caretRelative: Math.abs(box.top - geometry.top) <= geometry.height,
        occlusionRejected,
      };
    });
    check(previewProof.initiallyVisible, "Real Chromium rejected a visible product preview");
    check(previewProof.fullyInViewport, "Product preview escaped the real Chromium viewport");
    check(previewProof.rightEdgeClamped, "Product preview did not clamp at the right edge");
    check(previewProof.caretRelative, "Product preview was not anchored at the measured caret");
    check(previewProof.occlusionRejected, "Product preview remained armed while occluded");
    await page.evaluate(mainWorldSource);
    const snapshot = await page.evaluate(() =>
      globalThis.__BADI_PRODUCT_MAIN_WORLD_PROOF__.readDillingerMonacoSnapshotInMainWorld(),
    );
    check(snapshot !== null, "The exact product snapshot function rejected focused Dillinger");
    check(
      snapshot.before === trigger && snapshot.after === "" && snapshot.offset === trigger.length,
      "The exact product snapshot did not bind the fixed phrase fixture",
    );
    const before = await structuralMonacoState(page, trigger);
    assertStructuralState(before, trigger.length, trigger.length, "before product edit");

    const applied = await page.evaluate(
      ({ expected, text }) =>
        globalThis.__BADI_PRODUCT_MAIN_WORLD_PROOF__.applyDillingerMonacoEditInMainWorld(
          expected,
          text,
        ),
      { expected: snapshot, text: completion },
    );
    check(applied === true, "The exact product MAIN-world edit did not apply");
    const acceptedValue = trigger + completion;
    const accepted = await structuralMonacoState(page, acceptedValue);
    assertStructuralState(accepted, acceptedValue.length, acceptedValue.length, "after product edit");

    await page.evaluate(() => {
      globalThis.monaco.editor.getEditors()[0].trigger("badi.product.proof", "undo", null);
    });
    const undone = await structuralMonacoState(page, trigger);
    assertStructuralState(undone, trigger.length, trigger.length, "after Monaco undo");

    await page.evaluate(() => {
      globalThis.monaco.editor.getEditors()[0].trigger("badi.product.proof", "redo", null);
    });
    const redone = await structuralMonacoState(page, acceptedValue);
    assertStructuralState(redone, acceptedValue.length, acceptedValue.length, "after Monaco redo");

    await page.evaluate(() => {
      const editor = globalThis.monaco.editor.getEditors()[0];
      const model = globalThis.monaco.editor.getModels()[0];
      const restore = globalThis.__BADI_PRODUCT_MONACO_PROOF_RESTORE__;
      model.setValue(restore.value);
      if (restore.selection !== null) editor.setSelection(restore.selection);
      editor.setScrollPosition({ scrollTop: restore.scrollTop, scrollLeft: restore.scrollLeft });
      if (restore.focused) editor.focus();
      globalThis.__BADI_PRODUCT_MONACO_PROOF_RESTORED__ = model.getValue() === restore.value;
      delete globalThis.__BADI_PRODUCT_MONACO_PROOF_RESTORE__;
    });
    const restored = await page.evaluate(
      () => globalThis.__BADI_PRODUCT_MONACO_PROOF_RESTORED__ === true,
    );
    check(restored, "The Monaco proof did not restore its original disposable document state");
    check(extensionErrors.length === 0, `Extension errors: ${extensionErrors.join(" | ")}`);

    result = {
      record_version: 1,
      evidence_class: "real-target-main-world-contract",
      release_evidence: false,
      target: dillingerUrl,
      browser: (await command(chromiumExecutable, ["--version"])).stdout.trim(),
      browser_mode: settings.headed ? "headed" : "headless",
      extension: {
        id: extensionId,
        worker_started: true,
        popup_and_bundle_assets_loaded: true,
        optional_permission_declared: DILLINGER_OPTIONAL_HOST_PERMISSION,
        optional_permission_granted: false,
        dynamic_content_registered: false,
        command: contract.command.shortcut,
      },
      main_world: {
        implementation: "src/product/monaco-main-world.ts compiled by repository esbuild",
        phrase_case: "phrase_v1.thank-you",
        snapshot_exact_fixture: true,
        target_native_insert: applied,
        target_native_undo: undone.exactExpectedValue,
        target_native_redo: redone.exactExpectedValue,
        caret_offsets: [before.offset, accepted.offset, undone.offset, redone.offset],
        focus_preserved:
          accepted.editorFocus && undone.editorFocus && redone.editorFocus,
        scroll_preserved: [accepted, undone, redone].every(
          (state) => state.scrollTop === before.scrollTop && state.scrollLeft === before.scrollLeft,
        ),
        original_disposable_document_restored: restored,
      },
      preview: {
        right_and_bottom_edges_clamped: true,
        suggestion_only_caret_relative_overlay: true,
        fully_in_viewport: true,
        five_point_occlusion_rejected: true,
      },
      excluded: [
        "optional-permission user confirmation",
        "native-messaging broker chain",
        "content-script to worker routing",
      ],
    };
  } catch (error) {
    runError = error;
  } finally {
    const cleanupErrors = [];
    try {
      if (context !== null) await context.close();
    } catch (error) {
      cleanupErrors.push(error);
    }
    if (tempRoot !== null) {
      try {
        await waitForNoProcesses(tempRoot);
        await rm(tempRoot, { recursive: true, force: true });
        check(!(await exists(tempRoot)), "Disposable Monaco proof tree was not removed");
        await waitForNoProcesses(tempRoot);
      } catch (error) {
        cleanupErrors.push(error);
      }
    }
    if (cleanupErrors.length > 0) {
      runError = new AggregateError(
        runError === null ? cleanupErrors : [runError, ...cleanupErrors],
        "Product Monaco proof or cleanup failed",
      );
    } else if (result !== null) {
      result.cleanup = {
        disposable_profile_removed: true,
        browser_processes_remaining: 0,
      };
    }
  }
  if (runError !== null) throw runError;
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

await main();
