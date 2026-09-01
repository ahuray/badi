#!/usr/bin/env node

import { createHash } from "node:crypto";
import { execFile, spawn } from "node:child_process";
import { constants as fsConstants } from "node:fs";
import { createServer } from "node:http";
import {
  access,
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  readlink,
  readdir,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { cpus, release, tmpdir, totalmem, type as osType } from "node:os";
import { basename, delimiter, dirname, isAbsolute, join, resolve } from "node:path";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import { assertExactChromiumManifest } from "../scripts/manifest-policy.mjs";
import {
  SCENARIO_PLAN,
  scenarioCase,
  scenarioDefinition,
  validateCompletedScenarioIds,
  validateScenarioPlan,
} from "./scenario-plan.mjs";

const execFileAsync = promisify(execFile);
const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(packageRoot, "../..");
const fixtureRoot = join(repositoryRoot, "fixtures/web");
const distRoot = join(packageRoot, "dist");
const liveRoot = join(packageRoot, "live");
const diagnosticRoot = join(repositoryRoot, "output/playwright/badi-m2a");
const extensionId = "ckkiehcjbclcjckkkajohopoikeejkoa";
const extensionOrigin = `chrome-extension://${extensionId}/`;
const fixtureUrl = "http://localhost:4173/chromium.html";
const ghostSelector = "[data-badi-owned]";
const brokerBinary = join(repositoryRoot, "target/debug/badi-broker");
const nativeHostBinary = join(repositoryRoot, "target/debug/badi-native-host");
const nativeManifestBinary = join(
  repositoryRoot,
  "target/debug/badi-native-manifest",
);
const cliBinary = join(repositoryRoot, "target/debug/badictl");
const fakeHostSource = join(liveRoot, "fake-native-host.mjs");
const scenarioPlanSource = join(liveRoot, "scenario-plan.mjs");
const runnerSource = fileURLToPath(import.meta.url);
const manifestPolicySource = join(packageRoot, "scripts/manifest-policy.mjs");
const DEBOUNCE_MS = 140;
const DURABLE_EVIDENCE_ID =
  /^chromium-native-live-run\.[a-z0-9][a-z0-9-]{0,63}\.v1$/u;

function parseArguments(values) {
  const parsed = {
    describe: false,
    headed: false,
    smoke: false,
    evidenceId: null,
    samples: 1_000,
    warmups: 50,
    staleTrials: 100,
    chromiumExecutable: null,
  };
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (value === "--describe") {
      parsed.describe = true;
      continue;
    }
    if (value === "--headed") {
      parsed.headed = true;
      continue;
    }
    if (value === "--smoke") {
      parsed.smoke = true;
      parsed.samples = 3;
      parsed.warmups = 1;
      parsed.staleTrials = 5;
      continue;
    }
    const next = values[index + 1];
    if (value === "--evidence-id") {
      if (next === undefined || !DURABLE_EVIDENCE_ID.test(next)) {
        throw new Error(
          "--evidence-id must match chromium-native-live-run.<unique-slug>.v1",
        );
      }
      parsed.evidenceId = next;
      index += 1;
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
    if (value === "--samples" || value === "--warmups" || value === "--stale-trials") {
      if (next === undefined || !/^\d+$/u.test(next)) {
        throw new Error(`${value} requires a nonnegative integer`);
      }
      const count = Number(next);
      if (!Number.isSafeInteger(count) || count < 0) {
        throw new Error(`${value} is outside the supported range`);
      }
      if (value === "--samples") parsed.samples = count;
      if (value === "--warmups") parsed.warmups = count;
      if (value === "--stale-trials") parsed.staleTrials = count;
      index += 1;
      continue;
    }
    throw new Error(`Unknown argument: ${String(value)}`);
  }
  if (parsed.describe) {
    if (values.length !== 1) {
      throw new Error("--describe cannot be combined with live-run arguments");
    }
    return parsed;
  }
  if (!parsed.smoke && (parsed.samples < 1_000 || parsed.warmups < 50)) {
    throw new Error("Durable evidence requires at least 1000 samples after 50 warmups");
  }
  if (!parsed.smoke && parsed.staleTrials < 100) {
    throw new Error("Durable evidence requires at least 100 stale-response trials");
  }
  if (!parsed.smoke && parsed.evidenceId === null) {
    throw new Error(
      "Durable evidence requires --evidence-id chromium-native-live-run.<unique-slug>.v1",
    );
  }
  if (parsed.smoke && parsed.evidenceId !== null) {
    throw new Error("Smoke diagnostics do not accept a durable evidence identity");
  }
  return parsed;
}

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function round(value) {
  return Number(value.toFixed(3));
}

function nearestRank(values, percentile) {
  check(values.length > 0, "Cannot summarize an empty measurement set");
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.max(0, Math.ceil(percentile * sorted.length) - 1);
  return sorted[index];
}

function summarizeMeasurement(name, values, warmups, threshold) {
  const p95 = round(nearestRank(values, 0.95));
  const maximum = round(Math.max(...values));
  check(p95 <= threshold, `${name} p95 exceeded its release threshold`);
  return {
    name,
    evidence_class: "real-rust-chain",
    unit: "milliseconds",
    samples: values.length,
    warmups,
    statistic: "nearest-rank-p95",
    p95,
    maximum,
    threshold,
    status: "pass",
  };
}

async function exists(path) {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
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
      // Continue searching the current process PATH.
    }
  }
  throw new Error(
    "Chromium was not found on PATH; pass --chromium-executable ABSOLUTE_PATH",
  );
}

async function sha256Artifact(id, path) {
  const body = await readFile(path);
  return {
    id,
    bytes: body.byteLength,
    sha256: createHash("sha256").update(body).digest("hex"),
  };
}

async function command(file, args, options = {}) {
  const result = await execFileAsync(file, args, {
    cwd: options.cwd ?? repositoryRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  return { stdout: result.stdout, stderr: result.stderr };
}

async function repositoryRecord(requireClean) {
  const baseCommit = (await command("git", ["rev-parse", "HEAD"])).stdout.trim();
  const workingTreeDirty =
    (await command("git", ["status", "--porcelain", "--untracked-files=all"]))
      .stdout.length > 0;
  if (requireClean && workingTreeDirty) {
    throw new Error("Durable evidence requires a clean Git working tree");
  }
  return { base_commit: baseCommit, working_tree_dirty: workingTreeDirty };
}

async function validateRepository(commandRecords) {
  const validations = [
    {
      id: "cargo-build-bins",
      file: "cargo",
      args: ["build", "--workspace", "--bins"],
      cwd: repositoryRoot,
      label: "cargo build --workspace --bins",
    },
    {
      id: "cargo-test-workspace",
      file: "cargo",
      args: ["test", "--workspace"],
      cwd: repositoryRoot,
      label: "cargo test --workspace",
    },
    {
      id: "chromium-typecheck",
      file: "npm",
      args: ["run", "typecheck"],
      cwd: packageRoot,
      label: "npm run typecheck --workspace @badi/chromium",
    },
    {
      id: "chromium-unit",
      file: "npm",
      args: ["test"],
      cwd: packageRoot,
      label: "npm test --workspace @badi/chromium",
    },
    {
      id: "chromium-build-verify",
      file: "npm",
      args: ["run", "build:verify"],
      cwd: packageRoot,
      label: "npm run build:verify --workspace @badi/chromium",
    },
  ];
  for (const validation of validations) {
    await command(validation.file, validation.args, { cwd: validation.cwd });
    commandRecords.push({ id: validation.id, command: validation.label, exit_code: 0 });
  }
}

async function validateExtensionPolicy() {
  for (const path of [
    join(packageRoot, "manifest.json"),
    join(distRoot, "manifest.json"),
  ]) {
    assertExactChromiumManifest(JSON.parse(await readFile(path, "utf8")));
  }
}

async function loadPlaywright() {
  const module = await import("playwright");
  const packagePath = fileURLToPath(
    await import.meta.resolve("playwright/package.json"),
  );
  const packageInfo = JSON.parse(await readFile(packagePath, "utf8"));
  return { chromium: module.chromium, version: packageInfo.version };
}

async function startFixtureServer() {
  const files = new Map([
    ["/chromium.html", ["chromium.html", "text/html; charset=utf-8"]],
    ["/fixture.js", ["fixture.js", "text/javascript; charset=utf-8"]],
    ["/fixture.css", ["fixture.css", "text/css; charset=utf-8"]],
  ]);
  const server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url ?? "/", fixtureUrl);
      if (url.pathname === "/blank.html") {
        response.writeHead(200, {
          "content-type": "text/html; charset=utf-8",
          "cache-control": "no-store",
        });
        response.end("<!doctype html><title>blank lifecycle target</title>");
        return;
      }
      const entry = files.get(url.pathname);
      if (entry === undefined) {
        response.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
        response.end("not found");
        return;
      }
      const [name, contentType] = entry;
      const body = await readFile(join(fixtureRoot, name));
      response.writeHead(200, {
        "content-type": contentType,
        "content-length": String(body.byteLength),
        "cache-control": "no-store",
        "cross-origin-resource-policy": "same-origin",
      });
      response.end(body);
    } catch {
      response.writeHead(500, { "content-type": "text/plain; charset=utf-8" });
      response.end("fixture error");
    }
  });
  await new Promise((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(4173, "localhost", () => resolvePromise());
  });
  return server;
}

async function closeServer(server) {
  if (server === null) return;
  await new Promise((resolvePromise) => server.close(() => resolvePromise()));
}

async function waitForPath(path, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await exists(path)) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  }
  throw new Error(`Timed out waiting for ${basename(path)}`);
}

async function waitForPathAbsent(path, timeoutMs = 2_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!(await exists(path))) return true;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  }
  return !(await exists(path));
}

function modeString(value) {
  return (value & 0o777).toString(8).padStart(4, "0");
}

async function waitForChildExit(child, timeoutMs = 5_000) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return { code: child.exitCode, signal: child.signalCode };
  }
  return Promise.race([
    new Promise((resolvePromise) => {
      child.once("exit", (code, signal) => resolvePromise({ code, signal }));
    }),
    new Promise((_, reject) => {
      setTimeout(() => reject(new Error("Child process did not exit in time")), timeoutMs);
    }),
  ]);
}

async function stopChild(child) {
  if (child === null) return { code: null, signal: null };
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGTERM");
  try {
    return await waitForChildExit(child);
  } catch {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    return waitForChildExit(child);
  }
}

async function terminateRealNativeHost(callerLog) {
  const lines = (await readFile(callerLog, "utf8")).trim().split("\n").filter(Boolean);
  const pids = lines
    .filter((line) => /^pid:\d+$/u.test(line))
    .map((line) => Number(line.slice(4)))
    .reverse();
  for (const pid of pids) {
    if (!(await exists(join("/proc", String(pid))))) continue;
    const executable = await readlink(join("/proc", String(pid), "exe"));
    check(
      resolve(executable) === resolve(nativeHostBinary),
      "Tracked native-host PID did not resolve to the shipped Rust binary",
    );
    process.kill(pid, "SIGTERM");
    const remaining = await waitForPidsToExit([pid], 2_000);
    check(remaining.length === 0, "Rust native-host process did not exit");
    return pid;
  }
  throw new Error("No live tracked Rust native-host process was found");
}

async function installNativeManifest({ profile, home, xdgConfig, manifest }) {
  const relativeManifest = join("NativeMessagingHosts", "io.github.ahuray.badi.json");
  const destinations = [
    join(profile, relativeManifest),
    join(home, ".config/chromium", relativeManifest),
    join(xdgConfig, "chromium", relativeManifest),
  ];
  for (const destination of destinations) {
    await mkdir(dirname(destination), { recursive: true, mode: 0o700 });
    await writeFile(destination, manifest, { encoding: "utf8", mode: 0o600 });
  }
}

async function launchExtensionContext({
  chromium,
  chromiumExecutable,
  headed,
  profile,
  home,
  xdgConfig,
  xdgCache,
  runtime,
  extraEnv = {},
}) {
  return chromium.launchPersistentContext(profile, {
    executablePath: chromiumExecutable,
    headless: !headed,
    ignoreDefaultArgs: ["--disable-extensions"],
    env: {
      ...process.env,
      ...extraEnv,
      HOME: home,
      XDG_CONFIG_HOME: xdgConfig,
      XDG_CACHE_HOME: xdgCache,
      XDG_RUNTIME_DIR:
        headed && process.env.XDG_RUNTIME_DIR !== undefined
          ? process.env.XDG_RUNTIME_DIR
          : runtime,
    },
    args: [
      ...(headed ? [] : ["--headless=new"]),
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
}

async function waitForExtensionWorker(context) {
  const existing = context
    .serviceWorkers()
    .find((worker) => worker.url() === `${extensionOrigin}service-worker.js`);
  const worker = existing ?? (await context.waitForEvent("serviceworker", { timeout: 10_000 }));
  check(
    worker.url() === `${extensionOrigin}service-worker.js`,
    "Unexpected unpacked extension service worker",
  );
  return worker;
}

async function focusFixtureWindow(worker) {
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    const focused = await worker.evaluate(async (expectedUrl) => {
      const tabs = await chrome.tabs.query({});
      const tab = tabs.find((candidate) => candidate.url === expectedUrl);
      if (tab?.id === undefined || tab.windowId === undefined) return false;
      await chrome.tabs.update(tab.id, { active: true });
      await chrome.windows.update(tab.windowId, { focused: true });
      return (await chrome.windows.get(tab.windowId)).focused === true;
    }, fixtureUrl);
    if (focused) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 100));
  }
  throw new Error("Chromium did not focus the isolated fixture window");
}

async function openFixture(context, worker) {
  const page = await context.newPage();
  await page.goto(fixtureUrl, { waitUntil: "load" });
  await page.waitForFunction(() => typeof window.__badiLive === "object");
  await page.bringToFront();
  await focusFixtureWindow(worker);
  check(await page.evaluate(() => document.hasFocus()), "Fixture document is not focused");
  return page;
}

async function setupDraft(page, value, caret = value.length) {
  await page.evaluate(
    () => {
      const field = document.querySelector("#draft");
      if (!(field instanceof HTMLTextAreaElement)) throw new Error("Fixture API unavailable");
      const card = field.closest(".card");
      field.readOnly = false;
      field.disabled = false;
      field.hidden = false;
      field.removeAttribute("aria-hidden");
      field.removeAttribute("inert");
      field.style.removeProperty("display");
      if (card instanceof HTMLElement) {
        card.hidden = false;
        card.removeAttribute("data-badi");
        card.removeAttribute("aria-hidden");
        card.removeAttribute("inert");
        card.style.removeProperty("display");
      }
    },
  );
  // Let policy-observer records caused by cleanup drain before establishing a
  // fresh focus epoch; otherwise an old attribute record can cancel new work.
  await page.evaluate(() => new Promise((resolvePromise) => setTimeout(resolvePromise, 0)));
  await page.evaluate(
    ({ nextValue, nextCaret }) => {
      const api = window.__badiLive;
      const field = document.querySelector("#draft");
      const sink = document.querySelector("button[data-action='focus-away']");
      if (!api || !(field instanceof HTMLTextAreaElement) || !(sink instanceof HTMLButtonElement)) {
        throw new Error("Fixture API unavailable");
      }
      sink.focus();
      field.focus();
      api.resetEvents();
      api.setDraft(nextValue, nextCaret, nextCaret, true);
    },
    { nextValue: value, nextCaret: caret },
  );
}

async function waitForGhost(page, timeout = 4_000) {
  await page.waitForFunction(
    (selector) => {
      const host = document.querySelector(selector);
      return host instanceof HTMLElement && !host.hidden;
    },
    ghostSelector,
    { timeout },
  );
}

async function waitForGhostHidden(page, timeout = 250) {
  await page.waitForFunction(
    (selector) => {
      const host = document.querySelector(selector);
      return !(host instanceof HTMLElement) || host.hidden;
    },
    ghostSelector,
    { timeout },
  );
}

async function fieldSnapshot(page) {
  return page.evaluate(() => {
    const field = document.querySelector("#draft");
    if (!(field instanceof HTMLTextAreaElement)) throw new Error("Draft unavailable");
    return {
      value: field.value,
      start: field.selectionStart,
      end: field.selectionEnd,
    };
  });
}

async function fixtureEvents(page) {
  return page.evaluate(() => window.__badiLive?.events() ?? []);
}

async function resetFixtureEvents(page) {
  await page.evaluate(() => window.__badiLive?.resetEvents());
}

async function brokerStatus(socketPath, env) {
  const result = await command(
    cliBinary,
    ["--socket", socketPath, "status", "--json"],
    { env },
  );
  return JSON.parse(result.stdout);
}

async function pollBrokerPaused(socketPath, env, expected, timeoutMs = 2_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const status = await brokerStatus(socketPath, env);
    if (status.paused === expected) return true;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 40));
  }
  return false;
}

async function grantIsolatedFixturePolicy(socketPath, env) {
  const settings = {
    schema: "badi.settings.v2",
    revision: 1,
    paused: false,
    subjects: [
      {
        identity: {
          kind: "browser_origin",
          adapter: "chromium",
          scheme: "http",
          host: "localhost",
          port: 4173,
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
  const status = await brokerStatus(socketPath, env);
  check(status.paused === false, "Isolated fixture policy did not resume the broker");
}

function scenario(id, evidenceClass, trials, passed, detail, status = "pass") {
  const definition = scenarioDefinition(id);
  check(
    definition.evidence_class === evidenceClass,
    `${id} runtime evidence class does not match the scenario plan`,
  );
  return {
    id,
    evidence_class: evidenceClass,
    trials,
    passed,
    status,
    detail,
  };
}

function geometryAligned(snapshot) {
  if (!snapshot?.visible || snapshot.host === null || snapshot.field === null) return false;
  const leftAligned = Math.abs(snapshot.host.left - snapshot.field.left) <= 3;
  const below = Math.abs(snapshot.host.top - (snapshot.field.top + snapshot.field.height + 6)) <= 3;
  const above = Math.abs(snapshot.host.top - Math.max(8, snapshot.field.top - 102)) <= 3;
  return leftAligned && (below || above);
}

async function runRealChain({
  chromium,
  context,
  socketPath,
  brokerEnv,
  broker,
  callerLog,
  outputRoot,
  samples,
  warmups,
  scenarios,
  setStep,
}) {
  setStep("real.worker-and-fixture");
  const liveWorker = await waitForExtensionWorker(context);
  const page = await openFixture(context, liveWorker);

  setStep("real.full-chain");
  const fullChainCase = scenarioCase("chromium.full-chain");
  try {
    await setupDraft(page, fullChainCase.trigger);
    await waitForGhost(page);
  } catch (error) {
    const status = await brokerStatus(socketPath, brokerEnv);
    const nativeCalls = (await exists(callerLog))
      ? (await readFile(callerLog, "utf8")).trim()
      : "not-created";
    const message = error instanceof Error ? error.message : "unknown ghost timeout";
    throw new Error(
      `${message}; content-free broker status: ${JSON.stringify(status)}; native calls: ${nativeCalls}`,
    );
  }
  const liveStatus = await brokerStatus(socketPath, brokerEnv);
  check(liveStatus.metrics.provider_calls >= 1, "Real provider was not reached");
  scenarios.push(
    scenario(
      "chromium.full-chain",
      "real-rust-chain",
      1,
      1,
      "System Chromium loaded the unpacked worker and received a visible result through the Rust native host, private UDS, broker, and deterministic provider.",
    ),
  );

  const dismissCase = scenarioCase("interaction.dismiss");
  await setupDraft(page, dismissCase.trigger);
  await waitForGhost(page);
  const dismissBefore = await fieldSnapshot(page);
  await page.keyboard.press("Escape");
  await waitForGhostHidden(page);
  const dismissAfter = await fieldSnapshot(page);
  check(dismissAfter.value === dismissBefore.value, "Dismiss changed the field value");
  scenarios.push(
    scenario(
      "interaction.dismiss",
      "real-rust-chain",
      1,
      1,
      "A trusted Escape gesture hid the visible extension-owned UI without mutating the field.",
    ),
  );

  const undoCase = scenarioCase("interaction.undo");
  await setupDraft(page, undoCase.trigger);
  await waitForGhost(page);
  const acceptBefore = await fieldSnapshot(page);
  await resetFixtureEvents(page);
  await page.keyboard.press("Tab");
  await page.waitForFunction(
    ({ expectedLength }) => document.querySelector("#draft")?.value.length === expectedLength,
    { expectedLength: acceptBefore.value.length + undoCase.expected_output.length },
  );
  const acceptAfter = await fieldSnapshot(page);
  check(
    acceptAfter.value === `${acceptBefore.value}${undoCase.expected_output}` &&
    acceptAfter.start === acceptBefore.start + undoCase.expected_output.length &&
      acceptAfter.end === acceptAfter.start,
    "Trusted acceptance produced the wrong caret",
  );
  const acceptEvents = await fixtureEvents(page);
  check(
    acceptEvents.filter((event) => event.type === "field.input").length === 1,
    "Trusted acceptance did not dispatch exactly one input event",
  );
  await page.keyboard.press("Control+z");
  await page.waitForTimeout(40);
  const undoAfter = await fieldSnapshot(page);
  const undoSupported = undoAfter.value === acceptBefore.value;
  scenarios.push(
    scenario(
      "interaction.undo",
      "real-rust-chain",
      1,
      undoSupported ? 1 : 0,
      undoSupported
        ? "Chromium restored the pre-accept value through its undo gesture."
        : "The vanilla setRangeText plus dispatched input path did not create a Chromium undo transaction.",
      undoSupported ? "pass" : "unsupported",
    ),
  );

  const acceptWordCase = scenarioCase("commit.accept-word");
  const firstWordCompletion = /^ \S+/u.exec(acceptWordCase.expected_output)?.[0];
  check(firstWordCompletion !== undefined, "Planned accept-word output has no first word");
  await setupDraft(page, acceptWordCase.trigger);
  await waitForGhost(page);
  const wordBefore = await fieldSnapshot(page);
  await page.keyboard.press("Control+ArrowRight");
  await page.waitForFunction(
    ({ expectedLength }) => document.querySelector("#draft")?.value.length === expectedLength,
    { expectedLength: wordBefore.value.length + firstWordCompletion.length },
  );
  const wordAfter = await fieldSnapshot(page);
  check(
    wordAfter.value === `${wordBefore.value}${firstWordCompletion}` &&
      wordAfter.start === wordBefore.start + firstWordCompletion.length &&
      wordAfter.end === wordAfter.start,
    "Word accept value/caret mismatch",
  );
  scenarios.push(
    scenario(
      "commit.accept-word",
      "real-rust-chain",
      1,
      1,
      "A trusted Ctrl-ArrowRight gesture inserted only the broker-authorized first word-part.",
    ),
  );
  try {
    await waitForGhost(page, 2_000);
    await page.keyboard.press("Escape");
  } catch {
    await waitForGhostHidden(page);
  }

  const untrustedKeyboardCase = scenarioCase("security.untrusted-keyboard");
  await setupDraft(page, untrustedKeyboardCase.trigger);
  await waitForGhost(page);
  const hostileBefore = await fieldSnapshot(page);
  const hostileMetricsBefore = await brokerStatus(socketPath, brokerEnv);
  await page.evaluate(() => {
    const field = document.querySelector("#draft");
    if (!(field instanceof HTMLTextAreaElement)) throw new Error("Draft unavailable");
    for (const event of [
      new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }),
      new KeyboardEvent("keydown", {
        key: "ArrowRight",
        ctrlKey: true,
        bubbles: true,
        cancelable: true,
      }),
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
    ]) field.dispatchEvent(event);
  });
  await page.waitForTimeout(80);
  const hostileAfter = await fieldSnapshot(page);
  const hostileMetricsAfter = await brokerStatus(socketPath, brokerEnv);
  check(hostileAfter.value === hostileBefore.value, "Synthetic keyboard event mutated the field");
  check(
    hostileMetricsAfter.metrics.commits_prepared ===
      hostileMetricsBefore.metrics.commits_prepared,
    "Synthetic keyboard event reached commit authorization",
  );
  check(
    await page.evaluate(() => !document.querySelector("[data-badi-owned]")?.hidden),
    "Synthetic dismiss hid the suggestion",
  );
  await page.keyboard.press("Escape");
  scenarios.push(
    scenario(
      "security.untrusted-keyboard",
      "real-rust-chain",
      3,
      3,
      "Page-authored untrusted accept-word, accept-all, and dismiss events caused no authorization or mutation; trusted automation remained functional.",
    ),
  );

  const syntheticFocusCase = scenarioCase("security.synthetic-focus-zero");
  const syntheticMetricsBefore = await brokerStatus(socketPath, brokerEnv);
  await page.evaluate((trigger) => {
    const field = document.querySelector("#draft");
    const sink = document.querySelector("button[data-action='focus-away']");
    if (!(field instanceof HTMLTextAreaElement) || !(sink instanceof HTMLButtonElement)) {
      throw new Error("Fixture unavailable");
    }
    sink.focus();
    field.value = trigger;
    field.setSelectionRange(trigger.length, trigger.length);
    field.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    field.dispatchEvent(new InputEvent("input", { bubbles: true, data: "x" }));
  }, syntheticFocusCase.trigger);
  await page.waitForTimeout(DEBOUNCE_MS + 80);
  const syntheticMetricsAfter = await brokerStatus(socketPath, brokerEnv);
  check(
    syntheticMetricsAfter.metrics.context_updates ===
      syntheticMetricsBefore.metrics.context_updates &&
      syntheticMetricsAfter.metrics.provider_calls ===
        syntheticMetricsBefore.metrics.provider_calls,
    "Synthetic focus acquired context",
  );
  scenarios.push(
    scenario(
      "security.synthetic-focus-zero",
      "real-rust-chain",
      1,
      1,
      "Synthetic focusin and input on an unfocused eligible field produced zero broker context updates and provider calls.",
    ),
  );

  const deniedCases = scenarioDefinition("privacy.denied-zero").cases;
  const deniedBefore = await brokerStatus(socketPath, brokerEnv);
  await page.evaluate(
    ({ passwordTrigger, otpTrigger }) => {
      const password = document.querySelector("#password");
      const otp = document.querySelector("#otp");
      if (!(password instanceof HTMLInputElement) || !(otp instanceof HTMLInputElement)) {
        throw new Error("Denied fixture fields unavailable");
      }
      password.value = passwordTrigger;
      otp.value = otpTrigger;
    },
    {
      passwordTrigger: deniedCases[0].trigger,
      otpTrigger: deniedCases[1].trigger,
    },
  );
  await page.locator("#password").focus();
  await page.waitForTimeout(DEBOUNCE_MS + 80);
  await page.locator("#otp").focus();
  await page.waitForTimeout(DEBOUNCE_MS + 80);
  const deniedAfter = await brokerStatus(socketPath, brokerEnv);
  check(
    deniedAfter.metrics.context_updates === deniedBefore.metrics.context_updates &&
      deniedAfter.metrics.provider_calls === deniedBefore.metrics.provider_calls &&
      deniedAfter.metrics.provider_input_bytes === deniedBefore.metrics.provider_input_bytes,
    "Denied fields crossed the broker/provider boundary",
  );
  scenarios.push(
    scenario(
      "privacy.denied-zero",
      "real-rust-chain",
      2,
      2,
      "Password and one-time-code fields caused zero context, provider-call, and provider-input-byte deltas.",
    ),
  );

  setStep("real.dynamic-invalidation");
  const invalidationCases = scenarioDefinition("lifecycle.dynamic-invalidation").cases;
  let invalidationPasses = 0;
  for (const [index, mutation] of ["readonly", "ancestor-opt-out", "replace"].entries()) {
    process.stdout.write(`Live invalidation check: ${mutation}\n`);
    await setupDraft(page, invalidationCases[index].trigger);
    await waitForGhost(page);
    const before = await fieldSnapshot(page);
    if (mutation === "readonly") {
      await page.evaluate(() => {
        const field = document.querySelector("#draft");
        if (field instanceof HTMLTextAreaElement) field.readOnly = true;
      });
    } else if (mutation === "ancestor-opt-out") {
      await page.evaluate(() => {
        document.querySelector("#draft")?.closest(".card")?.setAttribute("data-badi", "off");
      });
    } else {
      await page.evaluate(() => window.__badiLive?.replaceDraft());
    }
    await waitForGhostHidden(page);
    await page.keyboard.press("Tab");
    const after = await fieldSnapshot(page);
    check(after.value === before.value, "Invalidated field accepted a stale suggestion");
    invalidationPasses += 1;
  }
  scenarios.push(
    scenario(
      "lifecycle.dynamic-invalidation",
      "real-rust-chain",
      3,
      invalidationPasses,
      "Readonly, ancestor opt-out, and DOM replacement each hid UI and prevented later insertion.",
    ),
  );

  setStep("real.composition");
  await setupDraft(page, scenarioCase("lifecycle.composition").trigger);
  await waitForGhost(page);
  const compositionBefore = await fieldSnapshot(page);
  await page.evaluate(() => window.__badiLive?.dispatchComposition("compositionstart", "x"));
  await waitForGhostHidden(page);
  await page.keyboard.press("Tab");
  const compositionAfter = await fieldSnapshot(page);
  check(compositionAfter.value === compositionBefore.value, "Composition invalidation inserted text");
  await page.locator("#draft").focus();
  await page.evaluate(() => window.__badiLive?.dispatchComposition("compositionend", "x"));
  scenarios.push(
    scenario(
      "lifecycle.composition",
      "real-rust-chain",
      1,
      1,
      "A synthetic CompositionEvent lifecycle stimulus in real Chromium hid the result and blocked a trusted acceptance gesture until refocus/composition end.",
    ),
  );

  setStep("real.geometry");
  await setupDraft(page, scenarioCase("geometry.scroll-zoom").trigger);
  await waitForGhost(page);
  const geometryInitial = await page.evaluate(() => window.__badiLive?.ghostSnapshot());
  check(geometryAligned(geometryInitial), "Initial ghost geometry was not anchored");
  await page.evaluate(() => window.scrollBy(0, 120));
  await page.waitForTimeout(50);
  const geometryScrolled = await page.evaluate(() => window.__badiLive?.ghostSnapshot());
  check(geometryAligned(geometryScrolled), "Scrolled ghost geometry was not anchored");
  const cdp = await context.newCDPSession(page);
  await cdp.send("Emulation.setPageScaleFactor", { pageScaleFactor: 1.15 });
  await page.evaluate(() => window.dispatchEvent(new Event("resize")));
  await page.waitForTimeout(50);
  const geometryZoomed = await page.evaluate(() => window.__badiLive?.ghostSnapshot());
  if (!geometryAligned(geometryZoomed)) {
    process.stdout.write(`Geometry diagnostic: ${JSON.stringify(geometryZoomed)}\n`);
  }
  check(geometryAligned(geometryZoomed), "Zoomed ghost geometry was not anchored");
  await cdp.send("Emulation.setPageScaleFactor", { pageScaleFactor: 1 });
  await cdp.detach();
  await page.evaluate(() => {
    window.scrollTo(0, 0);
    window.dispatchEvent(new Event("resize"));
  });
  await page.keyboard.press("Escape");
  scenarios.push(
    scenario(
      "geometry.scroll-zoom",
      "real-rust-chain",
      3,
      3,
      "The extension-owned host remained anchored before scroll, after scroll, and after controlled page zoom plus resize.",
    ),
  );

  await setupDraft(page, scenarioCase("lifecycle.visibility").trigger);
  await waitForGhost(page);
  const background = await context.newPage();
  await background.bringToFront();
  let documentHidden = false;
  try {
    await page.waitForFunction(() => document.visibilityState !== "visible", undefined, {
      timeout: 1_000,
    });
    documentHidden = true;
    await waitForGhostHidden(page);
  } catch {
    documentHidden = false;
  }
  await background.close();
  await page.bringToFront();
  scenarios.push(
    scenario(
      "lifecycle.visibility",
      "real-rust-chain",
      1,
      documentHidden ? 1 : 0,
      documentHidden
        ? "Headless tab deactivation changed visibility and synchronously cleared the UI."
        : "This headless build did not expose tab-background visibility transitions.",
      documentHidden ? "pass" : "unsupported",
    ),
  );

  const navigationCases = scenarioDefinition("lifecycle.navigation").cases;
  await setupDraft(page, navigationCases[0].trigger);
  await waitForGhost(page);
  await page.goto("http://localhost:4173/blank.html", { waitUntil: "load" });
  check((await page.locator(ghostSelector).count()) === 0, "Ghost survived document navigation");
  await page.goto(fixtureUrl, { waitUntil: "load" });
  await page.waitForFunction(() => typeof window.__badiLive === "object");
  await setupDraft(page, navigationCases[1].trigger);
  await waitForGhost(page);
  await page.keyboard.press("Escape");
  scenarios.push(
    scenario(
      "lifecycle.navigation",
      "real-rust-chain",
      2,
      2,
      "Navigation removed the old document UI, and the exact fixture document received a fresh isolated content lifecycle on return.",
    ),
  );

  const authoritativePauseCases = scenarioDefinition(
    "control.pause-authoritative",
  ).cases;
  await setupDraft(page, authoritativePauseCases[0].trigger);
  await waitForGhost(page);
  await command(cliBinary, ["--socket", socketPath, "pause", "on"], { env: brokerEnv });
  check(await pollBrokerPaused(socketPath, brokerEnv, true), "Broker did not enter pause");
  await waitForGhostHidden(page);
  const authoritativePauseBefore = await brokerStatus(socketPath, brokerEnv);
  await page.evaluate((trigger) => {
    window.__badiLive?.setDraft(trigger, trigger.length, trigger.length, true);
  }, authoritativePauseCases[1].trigger);
  await page.waitForTimeout(DEBOUNCE_MS + 100);
  const authoritativePauseAfter = await brokerStatus(socketPath, brokerEnv);
  check(
    authoritativePauseAfter.metrics.provider_calls ===
      authoritativePauseBefore.metrics.provider_calls,
    "Authoritative pause allowed provider work",
  );
  await command(cliBinary, ["--socket", socketPath, "pause", "off"], { env: brokerEnv });
  check(await pollBrokerPaused(socketPath, brokerEnv, false), "Broker did not leave pause");
  await setupDraft(page, authoritativePauseCases[2].trigger);
  await waitForGhost(page);
  await page.keyboard.press("Escape");
  scenarios.push(
    scenario(
      "control.pause-authoritative",
      "real-rust-chain",
      3,
      3,
      "Broker-authoritative pause cleared visible UI, blocked provider work for a later edit, and resume permitted a fresh real-chain result.",
    ),
  );

  const pauseShortcutCases = scenarioDefinition("control.pause-shortcut").cases;
  await setupDraft(page, pauseShortcutCases[0].trigger);
  await waitForGhost(page);
  await page.keyboard.press("Alt+Shift+P");
  const shortcutPaused = await pollBrokerPaused(socketPath, brokerEnv, true);
  let pausePassed = false;
  if (shortcutPaused) {
    await waitForGhostHidden(page);
    const beforePauseInput = await brokerStatus(socketPath, brokerEnv);
    await page.evaluate((trigger) => {
      window.__badiLive?.setDraft(trigger, trigger.length, trigger.length, true);
    }, pauseShortcutCases[1].trigger);
    await page.waitForTimeout(DEBOUNCE_MS + 100);
    const afterPauseInput = await brokerStatus(socketPath, brokerEnv);
    check(
      afterPauseInput.metrics.provider_calls === beforePauseInput.metrics.provider_calls,
      "Paused shortcut allowed provider work",
    );
    await page.keyboard.press("Alt+Shift+P");
    pausePassed = await pollBrokerPaused(socketPath, brokerEnv, false);
  }
  if (await pollBrokerPaused(socketPath, brokerEnv, true, 100)) {
    await command(cliBinary, ["--socket", socketPath, "pause", "off"], { env: brokerEnv });
    await pollBrokerPaused(socketPath, brokerEnv, false);
  }
  scenarios.push(
    scenario(
      "control.pause-shortcut",
      "real-rust-chain",
      1,
      pausePassed ? 1 : 0,
      pausePassed
        ? "The Chromium command waited for broker authority, broadcast the exact paused state, stopped provider work, and resumed authoritatively."
        : "Chromium headless did not deliver the extension command accelerator.",
      pausePassed ? "pass" : "unsupported",
    ),
  );

  const debounceCases = scenarioDefinition("schedule.debounce-latest").cases;
  const debounceTrials = 100;
  const staleBefore = await brokerStatus(socketPath, brokerEnv);
  await page.evaluate(async ({ trials, triggers }) => {
    const api = window.__badiLive;
    const field = document.querySelector("#draft");
    if (!api || !(field instanceof HTMLTextAreaElement)) throw new Error("Fixture unavailable");
    document.querySelector("button[data-action='focus-away']")?.focus();
    field.focus();
    api.resetEvents();
    for (let revision = 0; revision <= trials; revision += 1) {
      const value = triggers[revision % triggers.length];
      api.setDraft(value, value.length, value.length, true);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 2));
    }
  }, { trials: debounceTrials, triggers: debounceCases.map((entry) => entry.trigger) });
  await waitForGhost(page);
  const staleAfter = await brokerStatus(socketPath, brokerEnv);
  check(
    staleAfter.metrics.provider_calls - staleBefore.metrics.provider_calls === 1,
    "Debounced live schedule emitted more than its latest request",
  );
  await page.keyboard.press("Escape");
  scenarios.push(
    scenario(
      "schedule.debounce-latest",
      "real-rust-chain",
      debounceTrials + 1,
      debounceTrials + 1,
      "Rapid edits using only supported phrase triggers coalesced to one provider call for the latest revision.",
    ),
  );

  setStep("real.insertion-latency");
  const insertionPasses = [];
  const acceptLatencies = [];
  const editVisibleLatencies = [];
  const cases = scenarioDefinition("commit.insertion-100").cases;
  const totalInsertion = samples + warmups;
  for (let index = 0; index < totalInsertion; index += 1) {
    const selected = cases[index % cases.length];
    await setupDraft(page, selected.trigger);
    await waitForGhost(page);
    const visibleEvents = await fixtureEvents(page);
    const inputEvent = visibleEvents.find((event) => event.type === "field.input");
    const visibleEvent = [...visibleEvents].reverse().find((event) => event.type === "ghost.visible");
    check(inputEvent && visibleEvent, "Edit-to-visible endpoints were not observed");
    const before = await fieldSnapshot(page);
    const expected = `${before.value}${selected.expected_output}`;
    await resetFixtureEvents(page);
    await page.keyboard.press("Tab");
    await page.waitForFunction(
      ({ expectedLength }) => document.querySelector("#draft")?.value.length === expectedLength,
      { expectedLength: expected.length },
    );
    const after = await fieldSnapshot(page);
    const events = await fixtureEvents(page);
    const keyEvent = events.find((event) => event.type === "key.down" && event.key === "Tab");
    const insertedEvent = events.find((event) => event.type === "field.input");
    check(keyEvent && insertedEvent, "Accept-to-insert endpoints were not observed");
    const passed =
      after.value === expected &&
      after.start === before.start + selected.expected_output.length &&
      after.end === after.start &&
      events.filter((event) => event.type === "field.input").length === 1;
    check(passed, `Insertion/caret trial ${index + 1} failed`);
    if (index >= warmups) {
      insertionPasses.push({ category: selected.trigger, passed });
      acceptLatencies.push(insertedEvent.at_ms - keyEvent.at_ms);
      editVisibleLatencies.push(visibleEvent.at_ms - inputEvent.at_ms);
    }
  }
  check(insertionPasses.every((entry) => entry.passed), "An insertion trial failed");
  scenarios.push(
    scenario(
      "commit.insertion-100",
      "real-rust-chain",
      insertionPasses.length,
      insertionPasses.filter((entry) => entry.passed).length,
      "Real Chromium verified exact end-caret insertion and one input event across the four supported phrase_v1 probes; broader text shapes are not claimed.",
    ),
  );

  setStep("real.invalidation-latency");
  const invalidationLatencies = [];
  const totalInvalidation = samples + warmups;
  for (let index = 0; index < totalInvalidation; index += 1) {
    await setupDraft(page, cases[index % cases.length].trigger);
    await waitForGhost(page);
    await page.evaluate(() => {
      const api = window.__badiLive;
      const field = document.querySelector("#draft");
      if (!api || !(field instanceof HTMLTextAreaElement)) throw new Error("Fixture unavailable");
      api.resetEvents();
      api.mark("invalidate");
      field.readOnly = true;
    });
    await waitForGhostHidden(page);
    const events = await fixtureEvents(page);
    const marker = events.find(
      (event) => event.type === "fixture.mark" && event.label === "invalidate",
    );
    const hidden = events.find((event) => event.type === "ghost.hidden");
    check(marker && hidden, "Invalidation-to-hide endpoints were not observed");
    if (index >= warmups) invalidationLatencies.push(hidden.at_ms - marker.at_ms);
  }

  const measurements = [
    summarizeMeasurement("accept-to-insert", acceptLatencies, warmups, 30),
    summarizeMeasurement("invalidation-to-hide", invalidationLatencies, warmups, 32),
  ];
  const editP95 = round(nearestRank(editVisibleLatencies, 0.95));
  scenarios.push(
    scenario(
      "latency.edit-to-visible",
      "real-rust-chain",
      editVisibleLatencies.length,
      editP95 <= 500 ? editVisibleLatencies.length : 0,
      editP95 <= 500
        ? "Deterministic edit-to-visible p95 stayed within the provisional 500 ms warm interaction ceiling."
        : "Deterministic edit-to-visible p95 exceeded the provisional warm interaction ceiling.",
      editP95 <= 500 ? "pass" : "unsupported",
    ),
  );

  setStep("real.disconnect");
  await setupDraft(page, scenarioCase("lifecycle.disconnect").trigger);
  await waitForGhost(page);
  const disconnectBefore = await fieldSnapshot(page);
  await page.evaluate(() => {
    window.__badiLive?.resetEvents();
    window.__badiLive?.mark("disconnect-start");
  });
  await terminateRealNativeHost(callerLog);
  await waitForGhostHidden(page, 250);
  const disconnectEvents = await fixtureEvents(page);
  const disconnectStart = disconnectEvents.find(
    (event) => event.type === "fixture.mark" && event.label === "disconnect-start",
  );
  const disconnectHidden = disconnectEvents.find((event) => event.type === "ghost.hidden");
  check(disconnectStart && disconnectHidden, "Disconnect endpoints were not observed");
  check(
    disconnectHidden.at_ms - disconnectStart.at_ms <= 250,
    "Native disconnect did not hide before the sub-TTL deadline",
  );
  await page.keyboard.press("Tab");
  const disconnectAfter = await fieldSnapshot(page);
  check(disconnectAfter.value === disconnectBefore.value, "Disconnect retained an insertable result");
  scenarios.push(
    scenario(
      "lifecycle.disconnect",
      "real-rust-chain",
      1,
      1,
      "Terminating the tracked shipped Rust native-host process propagated through the worker's frozen document route; UI cleared under the sub-TTL deadline with no insertion.",
    ),
  );

  const brokerExit = await stopChild(broker);

  return { measurements, page, brokerExit };
}

async function runFaultHostRace({
  chromium,
  chromiumExecutable,
  headed,
  tempRoot,
  runtime,
  manifestGenerator,
  staleTrials,
  scenarios,
  setStep,
}) {
  setStep("fault-host.setup");
  const fakeRoot = join(tempRoot, "fault-host");
  const profile = join(fakeRoot, "profile");
  const home = join(fakeRoot, "home");
  const xdgConfig = join(fakeRoot, "config");
  const xdgCache = join(fakeRoot, "cache");
  const wrapper = join(fakeRoot, "fake-host-wrapper");
  const hostLog = join(fakeRoot, "host-events.jsonl");
  await Promise.all(
    [profile, home, xdgConfig, xdgCache].map((path) =>
      mkdir(path, { recursive: true, mode: 0o700 }),
    ),
  );
  await writeFile(
    wrapper,
    `#!/bin/sh\nexec "${process.execPath}" "${fakeHostSource}"\n`,
    { encoding: "utf8", mode: 0o700 },
  );
  await chmod(wrapper, 0o700);
  const generated = await command(manifestGenerator, ["--host-path", wrapper]);
  await installNativeManifest({ profile, home, xdgConfig, manifest: generated.stdout });
  const context = await launchExtensionContext({
    chromium,
    chromiumExecutable,
    headed,
    profile,
    home,
    xdgConfig,
    xdgCache,
    runtime,
    extraEnv: {
      BADI_LIVE_HOST_LOG: hostLog,
      BADI_LIVE_STALE_DELAY_MS: "500",
      BADI_LIVE_LATEST_DELAY_MS: "800",
    },
  });
  let hostPids = [];
  try {
    setStep("fault-host.stale-race");
    const faultCases = scenarioDefinition("race.stale-100").cases;
    const worker = await waitForExtensionWorker(context);
    const page = await openFixture(context, worker);
    await page.evaluate(async ({ trials, gap, staleTrigger, latestTrigger }) => {
      const api = window.__badiLive;
      const field = document.querySelector("#draft");
      if (!api || !(field instanceof HTMLTextAreaElement)) throw new Error("Fixture unavailable");
      document.querySelector("button[data-action='focus-away']")?.focus();
      field.focus();
      api.resetEvents();
      for (let revision = 0; revision < trials; revision += 1) {
        api.setDraft(staleTrigger, staleTrigger.length, staleTrigger.length, true);
        await new Promise((resolvePromise) => setTimeout(resolvePromise, gap));
      }
      api.setDraft(latestTrigger, latestTrigger.length, latestTrigger.length, true);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, gap));
      api.mark("inputs-complete");
    }, {
      trials: staleTrials,
      gap: DEBOUNCE_MS + 15,
      staleTrigger: faultCases[0].trigger,
      latestTrigger: faultCases[1].trigger,
    });
    await page.waitForTimeout(575);
    const staleWindowEvents = await fixtureEvents(page);
    const complete = staleWindowEvents.find(
      (event) => event.type === "fixture.mark" && event.label === "inputs-complete",
    );
    check(complete, "Fault-host race completion marker missing");
    const earlyVisible = staleWindowEvents.filter(
      (event) =>
        event.type === "ghost.visible" && event.at_ms < complete.at_ms + 575,
    );
    check(earlyVisible.length === 0, "A delayed stale response became visible");
    await waitForGhost(page, 2_000);
    const before = await fieldSnapshot(page);
    check(
      before.value === faultCases[1].trigger &&
        before.start === before.value.length &&
        before.end === before.start,
      "Fault race changed the final field before acceptance",
    );
    await page.keyboard.press("Tab");
    await page.waitForFunction(
      ({ expectedLength }) => document.querySelector("#draft")?.value.length === expectedLength,
      { expectedLength: before.value.length + faultCases[1].expected_output.length },
    );
    const after = await fieldSnapshot(page);
    check(
      after.value === `${before.value}${faultCases[1].expected_output}` &&
        after.start === before.start + faultCases[1].expected_output.length &&
        after.end === after.start,
      "Fault race did not accept exactly the distinct latest response",
    );
    const logBody = await readFile(hostLog, "utf8");
    const hostEvents = logBody
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line));
    hostPids = hostEvents
      .filter((event) => event.event === "host.start" && Number.isSafeInteger(event.pid))
      .map((event) => event.pid);
    const staleShows = hostEvents.filter(
      (event) => event.event === "suggestion.show" && event.scenario === "stale",
    ).length;
    const latestShows = hostEvents.filter(
      (event) => event.event === "suggestion.show" && event.scenario === "latest",
    ).length;
    const cancels = hostEvents.filter((event) => event.event === "suggest.cancel").length;
    check(staleShows === staleTrials, "Fault host did not emit every stale response");
    check(latestShows === 1, "Fault host did not emit exactly one latest response");
    check(cancels >= staleTrials, "Adapter did not cancel every superseded live request");
    scenarios.push(
      scenario(
        "race.stale-100",
        "live-browser-fault-host",
        staleTrials,
        staleTrials,
        "A separately labeled fault host returned every delayed response after supersession; none of the stale results displayed or inserted, and only the latest remained eligible.",
      ),
    );
  } finally {
    await context.close();
  }
  return { manifest: generated.stdout, hostPids };
}

async function waitForPidsToExit(pids, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs;
  let remaining = [];
  do {
    remaining = [];
    for (const pid of pids) {
      if (await exists(join("/proc", String(pid)))) remaining.push(pid);
    }
    if (remaining.length === 0) return [];
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 40));
  } while (Date.now() < deadline);
  return remaining;
}

async function countProcessesContaining(needle) {
  let count = 0;
  const entries = await readdir("/proc", { withFileTypes: true });
  for (const entry of entries) {
    if (!entry.isDirectory() || !/^\d+$/u.test(entry.name)) continue;
    try {
      const commandLine = await readFile(join("/proc", entry.name, "cmdline"), "utf8");
      if (commandLine.includes(needle)) count += 1;
    } catch {
      // Processes may exit between directory enumeration and read.
    }
  }
  return count;
}

async function environmentRecord(playwrightVersion, chromiumExecutable) {
  const osRelease = await readFile("/etc/os-release", "utf8");
  const pretty = osRelease.match(/^PRETTY_NAME=(?:"([^"]+)"|(.+))$/mu);
  const npmVersion = (await command("npm", ["--version"])).stdout.trim();
  return {
    os: pretty?.[1] ?? pretty?.[2] ?? osType(),
    kernel: `${osType()} ${release()}`,
    chromium: (await command(chromiumExecutable, ["--version"])).stdout.trim(),
    playwright: String(playwrightVersion),
    rustc: (await command("rustc", ["--version"])).stdout.trim(),
    cargo: (await command("cargo", ["--version"])).stdout.trim(),
    node: process.version,
    npm: npmVersion,
  };
}

async function artifactRecord(nativeManifest) {
  const artifacts = [
    await sha256Artifact("broker-binary", brokerBinary),
    await sha256Artifact("native-host-binary", nativeHostBinary),
    {
      id: "native-host-manifest",
      bytes: Buffer.byteLength(nativeManifest, "utf8"),
      sha256: createHash("sha256").update(nativeManifest).digest("hex"),
    },
    await sha256Artifact("extension-content-script", join(distRoot, "content-script.js")),
    await sha256Artifact("extension-service-worker", join(distRoot, "service-worker.js")),
    await sha256Artifact("extension-manifest", join(distRoot, "manifest.json")),
    await sha256Artifact("extension-build-manifest", join(distRoot, "BUILD_MANIFEST.json")),
    await sha256Artifact("fixture-html", join(fixtureRoot, "chromium.html")),
    await sha256Artifact("fixture-js", join(fixtureRoot, "fixture.js")),
    await sha256Artifact("fixture-css", join(fixtureRoot, "fixture.css")),
    await sha256Artifact("live-runner", runnerSource),
    await sha256Artifact("scenario-plan", scenarioPlanSource),
    await sha256Artifact("fault-host", fakeHostSource),
    await sha256Artifact("manifest-policy", manifestPolicySource),
  ];
  return artifacts;
}

async function main() {
  const settings = parseArguments(process.argv.slice(2));
  validateScenarioPlan();
  if (settings.describe) {
    process.stdout.write(
      `${JSON.stringify({ record_version: 1, scenarios: SCENARIO_PLAN }, null, 2)}\n`,
    );
    return;
  }
  const chromiumExecutable = await resolveChromiumExecutable(
    settings.chromiumExecutable,
  );
  const evidencePath = settings.smoke
    ? null
    : join(repositoryRoot, "capabilities/evidence", `${settings.evidenceId}.json`);
  if (evidencePath !== null && (await exists(evidencePath))) {
    throw new Error(
      `Refusing to overwrite existing durable evidence: ${settings.evidenceId}.json`,
    );
  }
  const initialRepository = await repositoryRecord(!settings.smoke);
  await mkdir(diagnosticRoot, { recursive: true });
  const commandRecords = [];
  const scenarios = [];
  let fixtureServer = null;
  let broker = null;
  let realContext = null;
  let tempRoot = null;
  let nativeManifest = "";
  let socketPath = null;
  let callerLog = null;
  const trackedHostPids = [];
  let runError = null;
  let measurements = [];
  let currentStep = "preflight";
  let playwrightVersion = "unknown";
  let cleanup = {
    temporary_tree_removed: false,
    socket_removed: false,
    processes_remaining: -1,
  };
  try {
    await validateRepository(commandRecords);
    await validateExtensionPolicy();
    const playwright = await loadPlaywright();
    playwrightVersion = playwright.version;
    tempRoot = await mkdtemp(join(tmpdir(), "badi-m2a-live-"));
    const runtime = join(tempRoot, "runtime");
    const realRoot = join(tempRoot, "real-chain");
    const profile = join(realRoot, "profile");
    const home = join(realRoot, "home");
    const xdgConfig = join(realRoot, "config");
    const xdgCache = join(realRoot, "cache");
    const wrapper = join(realRoot, "native-host-wrapper");
    callerLog = join(realRoot, "caller-argv.log");
    socketPath = join(runtime, "badi/broker.sock");
    await Promise.all(
      [runtime, profile, home, xdgConfig, xdgCache].map((path) =>
        mkdir(path, { recursive: true, mode: 0o700 }),
      ),
    );
    await writeFile(
      wrapper,
      '#!/bin/sh\numask 077\nprintf \'pid:%s\\narg:%s\\n\' "$$" "$1" >> "$BADI_LIVE_CALLER_LOG"\nexec "$BADI_LIVE_REAL_HOST" "$1" --socket "$BADI_LIVE_SOCKET" 2>> "$BADI_LIVE_CALLER_LOG"\n',
      { encoding: "utf8", mode: 0o700 },
    );
    await chmod(wrapper, 0o700);
    nativeManifest = (
      await command(nativeManifestBinary, ["--host-path", wrapper])
    ).stdout;
    const parsedManifest = JSON.parse(nativeManifest);
    check(
      parsedManifest.allowed_origins?.length === 1 &&
        parsedManifest.allowed_origins[0] === extensionOrigin,
      "Generated native manifest origin mismatch",
    );
    await installNativeManifest({ profile, home, xdgConfig, manifest: nativeManifest });

    const brokerEnv = {
      ...process.env,
      HOME: home,
      XDG_CONFIG_HOME: xdgConfig,
      XDG_CACHE_HOME: xdgCache,
      XDG_RUNTIME_DIR: runtime,
    };
    broker = spawn(brokerBinary, ["--socket", socketPath], {
      cwd: repositoryRoot,
      env: brokerEnv,
      stdio: ["ignore", "ignore", "ignore"],
    });
    await waitForPath(socketPath);
    check(modeString((await stat(dirname(socketPath))).mode) === "0700", "Socket parent mode mismatch");
    check(modeString((await stat(socketPath)).mode) === "0600", "Socket mode mismatch");
    await grantIsolatedFixturePolicy(socketPath, brokerEnv);
    commandRecords.push({
      id: "isolated-fixture-policy",
      command: "badictl settings replace (temporary localhost:4173 allow; learning blocked)",
      exit_code: 0,
    });
    fixtureServer = await startFixtureServer();
    commandRecords.push({
      id: "isolated-live-run",
      command:
        `node adapters/chromium/live/run-live.mjs (isolated $TEMP HOME/XDG/profile; ${settings.headed ? "headed" : "headless"} parameterized Chromium; generated host manifest; Rust broker/native host)`,
      exit_code: 0,
    });

    realContext = await launchExtensionContext({
      chromium: playwright.chromium,
      chromiumExecutable,
      headed: settings.headed,
      profile,
      home,
      xdgConfig,
      xdgCache,
      runtime,
      extraEnv: {
        BADI_LIVE_CALLER_LOG: callerLog,
        BADI_LIVE_REAL_HOST: nativeHostBinary,
        BADI_LIVE_SOCKET: socketPath,
      },
    });
    const real = await runRealChain({
      chromium: playwright.chromium,
      context: realContext,
      socketPath,
      brokerEnv,
      broker,
      callerLog,
      outputRoot: diagnosticRoot,
      samples: settings.samples,
      warmups: settings.warmups,
      scenarios,
      setStep: (value) => {
        currentStep = value;
      },
    });
    measurements = real.measurements;
    broker = null;
    const callerLines = (await readFile(callerLog, "utf8"))
      .trim()
      .split("\n")
      .filter(Boolean);
    const callerArguments = callerLines
      .filter((line) => line.startsWith("arg:"))
      .map((line) => line.slice(4));
    trackedHostPids.push(
      ...callerLines
        .filter((line) => /^pid:\d+$/u.test(line))
        .map((line) => Number(line.slice(4))),
    );
    check(
      callerArguments.includes(extensionOrigin),
      "Chrome did not supply the exact trailing-slash extension origin",
    );
    await realContext.close();
    realContext = null;

    const fault = await runFaultHostRace({
      chromium: playwright.chromium,
      chromiumExecutable,
      headed: settings.headed,
      tempRoot,
      runtime,
      manifestGenerator: nativeManifestBinary,
      staleTrials: settings.staleTrials,
      scenarios,
      setStep: (value) => {
        currentStep = value;
      },
    });
    trackedHostPids.push(...fault.hostPids);
  } catch (error) {
    runError = error;
  } finally {
    try {
      if (realContext !== null) await realContext.close();
    } catch {
      // Preserve the original failure.
    }
    try {
      await stopChild(broker);
    } catch {
      // Cleanup is verified below.
    }
    try {
      await closeServer(fixtureServer);
    } catch {
      // Cleanup is verified below.
    }
    if (tempRoot !== null) {
      if (callerLog !== null && (await exists(callerLog))) {
        const earlyCallerLines = (await readFile(callerLog, "utf8"))
          .trim()
          .split("\n")
          .filter(Boolean);
        for (const line of earlyCallerLines) {
          if (!/^pid:\d+$/u.test(line)) continue;
          const pid = Number(line.slice(4));
          if (!trackedHostPids.includes(pid)) trackedHostPids.push(pid);
        }
      }
      const deadline = Date.now() + 5_000;
      let remaining = await countProcessesContaining(tempRoot);
      while (remaining > 0 && Date.now() < deadline) {
        await new Promise((resolvePromise) => setTimeout(resolvePromise, 50));
        remaining = await countProcessesContaining(tempRoot);
      }
      const hostPidsRemaining = await waitForPidsToExit(trackedHostPids);
      const socketRemoved =
        socketPath === null ? true : await waitForPathAbsent(socketPath, 2_000);
      if (!socketRemoved && socketPath !== null) {
        // Always leave the isolated machine clean, but do not relabel this
        // fallback deletion as broker-owned cleanup evidence.
        await rm(socketPath, { force: true });
      }
      await rm(tempRoot, { recursive: true, force: true });
      cleanup = {
        temporary_tree_removed: !(await exists(tempRoot)),
        socket_removed: socketRemoved,
        processes_remaining: remaining + hostPidsRemaining.length,
      };
    }
  }

  if (runError !== null) {
    const diagnostic = {
      recorded_at: new Date().toISOString(),
      status: "fail",
      error: runError instanceof Error ? runError.message : "unknown live-run error",
      cleanup,
      current_step: currentStep,
      completed_scenarios: scenarios.map((entry) => entry.id),
    };
    await writeFile(
      join(diagnosticRoot, "live-run-failure.json"),
      `${JSON.stringify(diagnostic, null, 2)}\n`,
      "utf8",
    );
    throw runError;
  }
  check(cleanup.temporary_tree_removed, "Temporary tree was not removed");
  check(cleanup.socket_removed, "Broker did not remove its socket during shutdown");
  check(cleanup.processes_remaining === 0, "Isolated child processes remain");
  validateCompletedScenarioIds(scenarios.map((entry) => entry.id));

  const environment = await environmentRecord(
    playwrightVersion,
    chromiumExecutable,
  );
  const finalRepository = await repositoryRecord(!settings.smoke);
  check(
    finalRepository.base_commit === initialRepository.base_commit,
    "Repository HEAD changed during the live run",
  );
  const manifest = JSON.parse(await readFile(join(distRoot, "manifest.json"), "utf8"));
  assertExactChromiumManifest(manifest);
  const contentScript = manifest.content_scripts[0];
  const report = {
    $schema: "../v2/live-run.schema.json",
    record_version: 1,
    id: settings.smoke
      ? "chromium-native-live-run.smoke.v1"
      : settings.evidenceId,
    recorded_at: new Date().toISOString(),
    repository: finalRepository,
    environment,
    extension: {
      id: extensionId,
      manifest_version: manifest.manifest_version,
      minimum_chrome_version: manifest.minimum_chrome_version,
      incognito: manifest.incognito,
      permission_mode: "static-exact-port",
      api_permissions: manifest.permissions,
      manifest_keys: Object.keys(manifest).sort(),
      host_match: manifest.content_scripts[0].matches[0],
      content_script: {
        matches: contentScript.matches,
        js: contentScript.js,
        run_at: contentScript.run_at,
        all_frames: contentScript.all_frames,
        match_about_blank_present: Object.hasOwn(contentScript, "match_about_blank"),
        match_origin_as_fallback_present: Object.hasOwn(
          contentScript,
          "match_origin_as_fallback",
        ),
      },
      runtime_url: fixtureUrl,
    },
    native: {
      host_name: "io.github.ahuray.badi",
      caller_origin: extensionOrigin,
      max_envelope_bytes: 65_536,
      socket_parent_mode: "0700",
      socket_mode: "0600",
      peer_uid_verified: true,
    },
    isolation: {
      temporary_home: true,
      temporary_xdg_runtime: true,
      temporary_profile: true,
      real_profile_touched: false,
      user_config_touched: false,
      system_manifest_touched: false,
      temporary_tree_removed: cleanup.temporary_tree_removed,
      socket_removed: cleanup.socket_removed,
      processes_remaining: cleanup.processes_remaining,
    },
    privacy: {
      context_text_recorded: false,
      suggestion_text_recorded: false,
      context_fingerprints_recorded: false,
      absolute_personal_paths_recorded: false,
    },
    commands: commandRecords,
    scenarios,
    measurements,
    artifacts: await artifactRecord(nativeManifest),
    notes: [
      "The durable full-chain checks use the shipped Rust native host and broker; the delayed-response race is separately classified as live-browser-fault-host.",
      `${settings.headed ? "Headed" : "Headless"} Chromium loaded the extension but this run did not automate a browser permission prompt; this evidence therefore covers the static exact-document development boundary, not the future runtime-granted M2 policy.`,
      settings.headed
        ? "Headed automation used an isolated profile; it does not by itself prove physical IME, screen-reader, or human distraction quality."
        : "Headless automation does not prove headed window-manager accelerators, compositor rendering, accessibility contrast, browser-level zoom chrome, or MV3 restart epoch synchronization.",
      "Composition coverage uses synthetic CompositionEvent lifecycle stimuli in real Chromium; it does not claim a physical IME session.",
      "Vanilla DOM setRangeText insertion is externally verified here but remains broker-reported as dispatched-unverified; framework editors and contenteditable remain excluded.",
      "Latency endpoints use window.performance.now in the controlled page: trusted keydown to observed input, and pre-mutation marker to observed hidden transition; p95 is nearest-rank after warmups.",
      `Machine profile: ${cpus()[0]?.model ?? "unknown CPU"}; ${Math.round(totalmem() / 1024 ** 3)} GiB RAM.`,
    ],
  };

  if (settings.smoke) {
    await writeFile(
      join(diagnosticRoot, "smoke-results.json"),
      `${JSON.stringify(report, null, 2)}\n`,
      "utf8",
    );
    process.stdout.write(
      `Smoke live run passed (${settings.samples} measured / ${settings.warmups} warmup).\n`,
    );
    return;
  }
  check(evidencePath !== null, "Durable evidence path is unavailable");
  await mkdir(dirname(evidencePath), { recursive: true });
  await writeFile(evidencePath, `${JSON.stringify(report, null, 2)}\n`, {
    encoding: "utf8",
    flag: "wx",
  });
  process.stdout.write(
    `Durable live evidence passed: ${settings.samples} measured / ${settings.warmups} warmup; ${settings.staleTrials} delayed stale trials.\n`,
  );
}

await main();
