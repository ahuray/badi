import { NativeBrokerClient } from "./native-client";
import {
  EXPECTED_FIXTURE_ORIGIN,
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
  const options: { frameId: number; documentId?: string } = {
    frameId: route.frameId,
    ...(route.documentId === undefined ? {} : { documentId: route.documentId }),
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
    kind: "omatype.commit.revoke.v1",
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
    kind: "omatype.suggestion.clear.v1",
    event,
  });
});

broker.setDisconnectHandler(() => {
  void broadcastToRegisteredRoutes({
    kind: "omatype.transport.disconnected.v1",
  }).catch(() => undefined);
});

async function handle(
  command: RuntimeCommand,
  sender: chrome.runtime.MessageSender,
): Promise<RuntimeReply> {
  switch (command.kind) {
    case "omatype.suggest.v1": {
      if (command.request.origin !== EXPECTED_FIXTURE_ORIGIN) {
        return { ok: false, error: "Suggestion origin does not match fixture" };
      }
      if (!sessionRoutes.bind(command.request.sessionId, sender)) {
        return { ok: false, error: "Suggestion session is bound to another document" };
      }
      return { ok: true, response: await broker.requestSuggestion(command.request) };
    }
    case "omatype.cancel.v1":
      if (!sessionRoutes.matches(command.request.sessionId, sender)) {
        return { ok: false, error: "Suggestion session route mismatch" };
      }
      await broker.cancelSuggestion(command.request);
      return { ok: true };
    case "omatype.dismiss.v1":
      if (!sessionRoutes.matches(command.address.sessionId, sender)) {
        return { ok: false, error: "Suggestion session route mismatch" };
      }
      await broker.dismissSuggestion(command.address);
      return { ok: true };
    case "omatype.commit.authorize.v1":
      if (!sessionRoutes.matches(command.request.sessionId, sender)) {
        return { ok: false, error: "Suggestion session route mismatch" };
      }
      return { ok: true, response: await broker.authorizeCommit(command.request) };
    case "omatype.commit.result.v1":
      if (!sessionRoutes.matches(command.notice.sessionId, sender)) {
        return { ok: false, error: "Suggestion session route mismatch" };
      }
      await broker.reportCommit(command.notice);
      return { ok: true };
  }
}

chrome.runtime.onMessage.addListener((message: unknown, sender, sendResponse) => {
  if (!isRuntimeCommand(message)) {
    return false;
  }
  if (!isTrustedFixtureSender(sender, chrome.runtime.id)) {
    sendResponse({ ok: false, error: "Untrusted Omatype message sender" } satisfies RuntimeReply);
    return false;
  }
  void handle(message, sender).then(
    (reply) => sendResponse(reply),
    (error: unknown) => {
      const detail = error instanceof Error ? error.message : "Unknown native broker error";
      sendResponse({ ok: false, error: detail } satisfies RuntimeReply);
    },
  );
  return true;
});

chrome.tabs.onRemoved.addListener((tabId) => {
  sessionRoutes.deleteTab(tabId);
});

chrome.commands.onCommand.addListener((command) => {
  if (command !== "toggle-pause") {
    return;
  }
  void (async () => {
    const paused = await broker.globalControl("pause_toggle");
    const action = paused ? "pause" : "resume";
    await broadcastToRegisteredRoutes({ kind: "omatype.control.v1", action });
  })().catch(() => undefined);
});
