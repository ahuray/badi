#!/usr/bin/env node

import { appendFileSync } from "node:fs";

const MAX_FRAME_BYTES = 65_536;
const logPath = process.env["BADI_LIVE_HOST_LOG"];
const normalDelayMs = Number(process.env["BADI_LIVE_NORMAL_DELAY_MS"] ?? "8");
const staleDelayMs = Number(process.env["BADI_LIVE_STALE_DELAY_MS"] ?? "500");
const latestDelayMs = Number(process.env["BADI_LIVE_LATEST_DELAY_MS"] ?? "800");
const disconnectDelayMs = Number(
  process.env["BADI_LIVE_DISCONNECT_DELAY_MS"] ?? "250",
);

let input = Buffer.alloc(0);
let sequence = 0;
let paused = false;
const contexts = new Map();
const suggestions = new Map();
const timers = new Set();

function monoMs() {
  return Number(process.hrtime.bigint() / 1_000_000n);
}

function log(event, detail = {}) {
  if (!logPath) return;
  appendFileSync(
    logPath,
    `${JSON.stringify({ event, mono_ms: monoMs(), ...detail })}\n`,
    { encoding: "utf8", mode: 0o600 },
  );
}

function coordinates(frame) {
  return {
    session_id: frame.session_id,
    focus_epoch: frame.focus_epoch,
    revision: frame.revision,
  };
}

function coordinateKey(frame) {
  return `${frame.session_id}|${frame.focus_epoch}|${frame.revision}`;
}

function send(frame) {
  const body = Buffer.from(JSON.stringify(frame), "utf8");
  if (body.byteLength > MAX_FRAME_BYTES) {
    throw new Error("Fake host attempted an oversized native frame");
  }
  const header = Buffer.allocUnsafe(4);
  header.writeUInt32LE(body.byteLength, 0);
  process.stdout.write(Buffer.concat([header, body]));
}

function later(delayMs, callback) {
  const timer = setTimeout(() => {
    timers.delete(timer);
    callback();
  }, delayMs);
  timers.add(timer);
}

function sendError(id, code) {
  send({
    v: 1,
    ...(typeof id === "string" ? { id } : {}),
    type: "error",
    mono_ms: monoMs(),
    payload: { code },
  });
}

function handleHello(frame) {
  send({
    v: 1,
    id: "chromium.hello",
    type: "hello.ack",
    mono_ms: monoMs(),
    payload: {
      selected_v: 1,
      connection_id: `c:live-${process.pid}`,
      enabled_capabilities: [
        "context",
        "suggestion",
        "commit.dispatched_unverified",
        "control",
        "health",
      ],
      max_frame_bytes: MAX_FRAME_BYTES,
      max_before_chars: 512,
      max_after_chars: 128,
      max_suggestion_chars: 64,
      max_suggestion_words: 8,
      paused,
    },
  });
  log("hello.ack");
}

function handleSuggestion(frame) {
  if (paused) {
    sendError(frame.id, "paused");
    return;
  }
  const context = contexts.get(coordinateKey(frame));
  if (!context || context.fingerprint !== frame.payload?.fingerprint) {
    sendError(frame.id, "no_context");
    return;
  }

  const isStaleTrial = context.before.startsWith("stale-live-");
  const isLatestTrial = context.before === "stale-live-final";
  const isDisconnectTrial = context.before.startsWith("disconnect-live-");
  const text = isLatestTrial ? " latest" : " live";
  const suggestionId = `live-${++sequence}`;
  const suggestion = {
    id: frame.id,
    ...coordinates(frame),
    fingerprint: frame.payload.fingerprint,
    suggestion_id: suggestionId,
    text,
    accept_word: text,
  };
  suggestions.set(suggestionId, suggestion);
  const delayMs = isLatestTrial ? latestDelayMs : isStaleTrial ? staleDelayMs : normalDelayMs;
  later(delayMs, () => {
    send({
      v: 1,
      id: frame.id,
      type: "suggestion.show",
      ...coordinates(frame),
      mono_ms: monoMs(),
      payload: {
        fingerprint: frame.payload.fingerprint,
        suggestion_id: suggestionId,
        text,
        accept_word: text,
        ttl_ms: 600,
        provider: "phrase_v1",
      },
    });
    log("suggestion.show", {
      revision: frame.revision,
      scenario: isLatestTrial
        ? "latest"
        : isStaleTrial
          ? "stale"
          : isDisconnectTrial
            ? "disconnect"
            : "normal",
      output_bytes: Buffer.byteLength(text, "utf8"),
    });
    if (isDisconnectTrial) {
      later(disconnectDelayMs, () => {
        log("host.disconnect", { reason: "fixture-requested" });
        process.exit(0);
      });
    }
  });
}

function handleAddressedControl(frame) {
  const action = frame.payload?.action;
  const suggestion = suggestions.get(frame.payload?.suggestion_id);
  if (!suggestion || suggestion.fingerprint !== frame.payload?.fingerprint) {
    sendError(frame.id, "no_suggestion");
    return;
  }
  if (action === "dismiss") {
    send({
      v: 1,
      id: frame.id,
      type: "suggestion.clear",
      ...coordinates(frame),
      mono_ms: monoMs(),
      payload: {
        fingerprint: frame.payload.fingerprint,
        suggestion_id: frame.payload.suggestion_id,
        reason: "dismissed",
      },
    });
    log("control.dismiss", { revision: frame.revision });
    return;
  }
  if (action !== "accept_word" && action !== "accept_all") {
    sendError(frame.id, "invalid_message");
    return;
  }
  const acceptance = action === "accept_word" ? "word" : "all";
  const text = action === "accept_word" ? suggestion.accept_word : suggestion.text;
  send({
    v: 1,
    id: frame.id,
    type: "control.result",
    mono_ms: monoMs(),
    payload: { action, accepted: true, reason: "accepted", paused },
  });
  send({
    v: 1,
    id: frame.id,
    type: "commit.prepare",
    ...coordinates(frame),
    mono_ms: monoMs(),
    payload: {
      fingerprint: frame.payload.fingerprint,
      suggestion_id: frame.payload.suggestion_id,
      text,
      acceptance,
    },
  });
  log("commit.prepare", {
    revision: frame.revision,
    acceptance,
    output_bytes: Buffer.byteLength(text, "utf8"),
  });
}

function handleGlobalControl(frame) {
  const action = frame.payload?.action;
  if (action === "pause") paused = true;
  else if (action === "resume") paused = false;
  else if (action === "pause_toggle") paused = !paused;
  else {
    sendError(frame.id, "invalid_message");
    return;
  }
  send({
    v: 1,
    id: frame.id,
    type: "control.result",
    mono_ms: monoMs(),
    payload: { action, accepted: true, reason: "accepted", paused },
  });
  log("control.global", { action, paused });
}

function handle(frame) {
  log("frame.in", {
    type: frame?.type,
    focus_epoch: frame?.focus_epoch,
    revision: frame?.revision,
  });
  if (!frame || frame.v !== 1 || typeof frame.type !== "string") {
    sendError(frame?.id, "invalid_frame");
    return;
  }
  switch (frame.type) {
    case "hello":
      handleHello(frame);
      break;
    case "session.open":
      break;
    case "context.changed":
      contexts.set(coordinateKey(frame), {
        fingerprint: frame.payload?.fingerprint,
        before: frame.payload?.before ?? "",
      });
      break;
    case "suggest.request":
      handleSuggestion(frame);
      break;
    case "suggest.cancel":
      log("suggest.cancel", { revision: frame.revision });
      break;
    case "control.request":
      if (typeof frame.session_id === "string") handleAddressedControl(frame);
      else handleGlobalControl(frame);
      break;
    case "commit.result":
      log("commit.result", {
        revision: frame.revision,
        status: frame.payload?.status,
        suggestion_id: frame.payload?.suggestion_id,
      });
      break;
    default:
      sendError(frame.id, "invalid_message");
  }
}

process.stdin.on("data", (chunk) => {
  input = Buffer.concat([input, chunk]);
  while (input.byteLength >= 4) {
    const length = input.readUInt32LE(0);
    if (length < 1 || length > MAX_FRAME_BYTES) {
      log("host.error", { reason: "invalid-frame-length", length });
      process.exit(2);
    }
    if (input.byteLength < length + 4) return;
    const body = input.subarray(4, length + 4);
    input = input.subarray(length + 4);
    try {
      handle(JSON.parse(body.toString("utf8")));
    } catch (error) {
      log("host.error", {
        reason: error instanceof Error ? error.message : "unknown",
      });
      process.exit(2);
    }
  }
});

process.stdin.on("end", () => {
  log("host.end");
  for (const timer of timers) clearTimeout(timer);
});

process.stdout.on("error", (error) => {
  if (error?.code === "EPIPE") process.exit(0);
  throw error;
});

log("host.start", { pid: process.pid });
