import { NativeBrokerClient } from "./native-client";
import {
  EXPECTED_FIXTURE_ORIGIN,
  EXPECTED_FIXTURE_URL,
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
const POLICY_LIFETIME_PORT = "badi-policy-lifetime-v1";

chrome.runtime.onConnect.addListener((port) => {
  if (
    port.name !== POLICY_LIFETIME_PORT ||
    port.sender === undefined ||
    !isTrustedFixtureBootstrapSender(port.sender, chrome.runtime.id)
  ) {
    port.disconnect();
    return;
  }
  const sender = port.sender;
  port.onDisconnect.addListener(() => {
    const sessionIds = sessionRoutes.deleteDocument(sender);
    void Promise.allSettled(sessionIds.map((sessionId) => broker.closeSession(sessionId)));
  });
});

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

function pauseWasApplied(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    Object.keys(value).length === 2 &&
    "applied" in value &&
    value.applied === true &&
    "paused" in value &&
    value.paused === true
  );
}

function policyWasApplied(value: unknown, authorityEpoch: number): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    Object.keys(value).length === 2 &&
    "applied" in value &&
    value.applied === true &&
    "authorityEpoch" in value &&
    value.authorityEpoch === authorityEpoch
  );
}

async function retireRoute(sessionId: string, route: TrustedSessionRoute): Promise<void> {
  if (!sessionRoutes.delete(sessionId, route)) return;
  await broker.closeSession(sessionId).catch(() => undefined);
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

broker.setAuthorityChangedHandler(async () => {
  const entries = sessionRoutes.entries();
  const pauses = await Promise.allSettled(
    entries.map(({ route }) =>
      sendToRoute(route, { kind: "badi.control.v1", action: "pause" }),
    ),
  );
  const live = entries.filter((_entry, index) => {
    const delivery = pauses[index];
    return delivery?.status === "fulfilled" && pauseWasApplied(delivery.value);
  });
  await Promise.allSettled(
    entries
      .filter((_entry, index) => !live.includes(_entry))
      .map(({ sessionId, route }) => retireRoute(sessionId, route)),
  );
  const refreshed = await Promise.all(
    live.map(async ({ sessionId, route }) => ({
      sessionId,
      route,
      policy: await broker.resolvePolicy(sessionId, route.origin),
    })),
  );
  const deliveries = await Promise.allSettled(
    refreshed.map(({ route, policy }) =>
      sendToRoute(route, { kind: "badi.policy.v1", policy }),
    ),
  );
  await Promise.allSettled(
    refreshed
      .filter(({ policy }, index) => {
        const delivery = deliveries[index];
        return !(
          delivery?.status === "fulfilled" &&
          policyWasApplied(delivery.value, policy.authorityEpoch)
        );
      })
      .map(({ sessionId, route }) => retireRoute(sessionId, route)),
  );
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
      const route = sessionRoutes.get(command.sessionId);
      if (route === null) {
        return { ok: false, error: "Bootstrap route is unavailable" };
      }
      const bootstrap = await broker.bootstrap(command.sessionId, route.origin);
      if (!sessionRoutes.matches(command.sessionId, sender)) {
        return { ok: false, error: "Bootstrap session route was displaced" };
      }
      return { ok: true, paused: bootstrap.paused, policy: bootstrap.policy };
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

async function senderDocumentIsCurrentlyActive(
  sender: chrome.runtime.MessageSender,
): Promise<boolean> {
  const tabId = sender.tab?.id;
  const windowId = sender.tab?.windowId;
  if (typeof tabId !== "number" || typeof windowId !== "number") return false;
  try {
    const [tab, window] = await Promise.all([
      chrome.tabs.get(tabId),
      chrome.windows.get(windowId),
    ]);
    return (
      tab.active === true &&
      tab.incognito === false &&
      tab.discarded === false &&
      tab.frozen === false &&
      tab.url === EXPECTED_FIXTURE_URL &&
      window.focused === true
    );
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
  if (message.kind === "badi.bootstrap.v1") {
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
    !contentFreeBootstrap &&
    !closingBoundSession &&
    !sessionRoutes.matches(routeSessionId, sender)
  ) {
    sendResponse({
      ok: false,
      error: "Badi policy bootstrap is required for this document",
    } satisfies RuntimeReply);
    return false;
  }
  void Promise.allSettled(
    displacedSessionIds.map((sessionId) => broker.closeSession(sessionId)),
  ).then(() =>
    closingBoundSession || contentFreeBootstrap
      ? true
      : senderDocumentIsCurrentlyActive(sender),
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
    await broker.globalControl("pause_toggle");
  })().catch(() => undefined);
});
