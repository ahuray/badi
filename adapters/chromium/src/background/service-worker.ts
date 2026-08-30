import { NativeBrokerClient } from "./native-client";
import {
  EXPECTED_FIXTURE_ORIGIN,
  isTrustedFixtureBootstrapSender,
  isTrustedFixtureSender,
} from "./fixture-boundary";
import {
  isRuntimeCommand,
  type ContentControlMessage,
  type RuntimeCommand,
  type RuntimeReply,
} from "../shared/runtime-messages";
import {
  SessionRouteRegistry,
  type TrustedSessionRoute,
} from "./session-routes";

const broker = new NativeBrokerClient({
  connectNative: (hostName) => chrome.runtime.connectNative(hostName),
});

const sessionRoutes = new SessionRouteRegistry();

function sendToRoute(
  route: TrustedSessionRoute,
  message: ContentControlMessage,
): Promise<unknown> {
  const options: { frameId: number; documentId: string } = {
    frameId: route.frameId,
    documentId: route.documentId,
  };
  return chrome.tabs.sendMessage(route.tabId, message, options);
}

function sendToSession(sessionId: string, message: ContentControlMessage): void {
  const route = sessionRoutes.get(sessionId);
  if (route === null) return;
  void sendToRoute(route, message).catch(() => undefined);
}

async function broadcastToRegisteredRoutes(
  message: ContentControlMessage,
): Promise<void> {
  await Promise.allSettled(
    sessionRoutes.snapshot().map((route) => sendToRoute(route, message)),
  );
}

broker.setCommitRevocationHandler((request) => {
  const message: ContentControlMessage = {
    kind: "badi.commit.revoke.v1",
    address: {
      requestId: request.requestId,
      sessionId: request.sessionId,
      focusEpoch: request.focusEpoch,
      revision: request.revision,
      monotonicMs: request.monotonicMs,
      fingerprint: request.fingerprint,
      suggestionId: request.suggestionId,
    },
  };
  sendToSession(request.sessionId, message);
});

broker.setSuggestionClearHandler((event) => {
  sendToSession(event.sessionId, {
    kind: "badi.suggestion.clear.v1",
    event,
  });
});

broker.setDisconnectHandler(() => {
  void broadcastToRegisteredRoutes({
    kind: "badi.transport.disconnected.v1",
  }).catch(() => undefined);
});

async function handle(
  command: RuntimeCommand,
  sender: chrome.runtime.MessageSender,
): Promise<RuntimeReply> {
  switch (command.kind) {
    case "badi.bootstrap.v1": {
      if (!sessionRoutes.matches(command.sessionId, sender)) {
        return { ok: false, error: "Bootstrap session route was displaced" };
      }
      const paused = await broker.bootstrap();
      if (!sessionRoutes.matches(command.sessionId, sender)) {
        return { ok: false, error: "Bootstrap session route was displaced" };
      }
      return { ok: true, paused };
    }
    case "badi.suggest.v1": {
      if (command.request.origin !== EXPECTED_FIXTURE_ORIGIN) {
        return { ok: false, error: "Suggestion origin does not match fixture" };
      }
      if (!sessionRoutes.matches(command.request.sessionId, sender)) {
        return { ok: false, error: "Suggestion session is bound to another document" };
      }
      return { ok: true, response: await broker.requestSuggestion(command.request) };
    }
    case "badi.cancel.v1":
      if (!sessionRoutes.matches(command.request.sessionId, sender)) {
        return { ok: false, error: "Suggestion session route mismatch" };
      }
      await broker.cancelSuggestion(command.request);
      return { ok: true };
    case "badi.session.close.v1":
      if (!sessionRoutes.matches(command.sessionId, sender)) {
        return { ok: false, error: "Suggestion session route mismatch" };
      }
      await broker.closeSession(command.sessionId);
      return { ok: true };
    case "badi.dismiss.v1":
      if (!sessionRoutes.matches(command.address.sessionId, sender)) {
        return { ok: false, error: "Suggestion session route mismatch" };
      }
      await broker.dismissSuggestion(command.address);
      return { ok: true };
    case "badi.commit.authorize.v1":
      if (!sessionRoutes.matches(command.request.sessionId, sender)) {
        return { ok: false, error: "Suggestion session route mismatch" };
      }
      return { ok: true, response: await broker.authorizeCommit(command.request) };
    case "badi.commit.result.v1":
      if (!sessionRoutes.matches(command.notice.sessionId, sender)) {
        return { ok: false, error: "Suggestion session route mismatch" };
      }
      await broker.reportCommit(command.notice);
      return { ok: true };
  }
}

async function senderWindowIsFocused(
  sender: chrome.runtime.MessageSender,
): Promise<boolean> {
  const windowId = sender.tab?.windowId;
  if (typeof windowId !== "number") return false;
  try {
    const window = await chrome.windows.get(windowId);
    return window.focused === true;
  } catch {
    return false;
  }
}

function commandSessionId(command: RuntimeCommand): string {
  switch (command.kind) {
    case "badi.bootstrap.v1":
    case "badi.session.close.v1":
      return command.sessionId;
    case "badi.suggest.v1":
    case "badi.cancel.v1":
      return command.request.sessionId;
    case "badi.dismiss.v1":
      return command.address.sessionId;
    case "badi.commit.authorize.v1":
      return command.request.sessionId;
    case "badi.commit.result.v1":
      return command.notice.sessionId;
  }
}

chrome.runtime.onMessage.addListener((message: unknown, sender, sendResponse) => {
  if (!isRuntimeCommand(message)) {
    return false;
  }
  const closingBoundSession =
    message.kind === "badi.session.close.v1" &&
    sender.id === chrome.runtime.id &&
    sessionRoutes.matches(message.sessionId, sender);
  const contentFreeBootstrap = message.kind === "badi.bootstrap.v1";
  const trustedExactDocument = contentFreeBootstrap
    ? isTrustedFixtureBootstrapSender(sender, chrome.runtime.id)
    : isTrustedFixtureSender(sender, chrome.runtime.id);
  if (!trustedExactDocument && !closingBoundSession) {
    sendResponse({ ok: false, error: "Untrusted Badi message sender" } satisfies RuntimeReply);
    return false;
  }
  let displacedSessionIds: readonly string[] = [];
  const routeSessionId = commandSessionId(message);
  if (
    message.kind === "badi.bootstrap.v1" ||
    (!closingBoundSession && !sessionRoutes.matches(routeSessionId, sender))
  ) {
    // MV3 may restart this worker while its content script remains alive.
    // Re-register only after the ordinary command passed the stricter active
    // exact-document sender check above; the broker still independently
    // validates coordinates, policy, and commit authority.
    const subscription = sessionRoutes.subscribe(routeSessionId, sender);
    if (subscription === null) {
      sendResponse({
        ok: false,
        error: "Badi session is bound to another document",
      } satisfies RuntimeReply);
      return false;
    }
    displacedSessionIds = subscription.displacedSessionIds;
  }
  if (
    message.kind === "badi.suggest.v1" &&
    !sessionRoutes.matches(message.request.sessionId, sender)
  ) {
    sendResponse({
      ok: false,
      error: "Suggestion session is bound to another document",
    } satisfies RuntimeReply);
    return false;
  }
  void Promise.allSettled(
    displacedSessionIds.map((sessionId) => broker.closeSession(sessionId)),
  ).then(() =>
    closingBoundSession || contentFreeBootstrap
      ? true
      : senderWindowIsFocused(sender),
  ).then(
    (focused) =>
      focused
        ? handle(message, sender)
        : ({ ok: false, error: "Badi sender window is not focused" } satisfies RuntimeReply),
  ).then(
    (reply) => sendResponse(reply),
    (error: unknown) => {
      const detail = error instanceof Error ? error.message : "Unknown native broker error";
      sendResponse({ ok: false, error: detail } satisfies RuntimeReply);
    },
  );
  return true;
});

chrome.tabs.onRemoved.addListener((tabId) => {
  const sessionIds = sessionRoutes.deleteTab(tabId);
  void Promise.allSettled(sessionIds.map((sessionId) => broker.closeSession(sessionId)));
});

chrome.commands.onCommand.addListener((command) => {
  if (command !== "toggle-pause") {
    return;
  }
  void (async () => {
    const paused = await broker.globalControl("pause_toggle");
    const action = paused ? "pause" : "resume";
    await broadcastToRegisteredRoutes({ kind: "badi.control.v1", action });
  })().catch(() => undefined);
});
