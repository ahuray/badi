import { NativeBrokerClient } from "../background/native-client";
import {
  SessionRouteRegistry,
  type TrustedSessionRoute,
} from "../background/session-routes";
import type { ContentControlMessage, RuntimeCommand, RuntimeReply } from "../shared/runtime-messages";
import { isRuntimeCommand } from "../shared/runtime-messages";
import { sanitizeSuggestion } from "../content/context";
import {
  applyDillingerMonacoEditInMainWorld,
  readDillingerMonacoSnapshotInMainWorld,
  type MonacoSnapshot,
} from "./monaco-main-world";
import { ProductCommitLinearizer } from "./commit-linearizer";
import {
  isProductBridgeCommand,
  isProductExtensionCommand,
  parseSnapshotReply,
  type ProductBridgeCommand,
  type ProductBridgeReply,
  type ProductControlMessage,
  type ProductExtensionReply,
} from "./messages";
import {
  hasDillingerAccess,
  permissionSetIncludesDillinger,
} from "./permissions";
import {
  DILLINGER_CONTENT_MATCH,
  DILLINGER_ORIGIN,
  DILLINGER_URL,
  PRODUCT_LIFETIME_PORT,
  isExactDillingerUrl,
  isTrustedDillingerSender,
} from "./target";

const PRODUCT_SCRIPT_ID = "badi-dillinger-product-v1";
const broker = new NativeBrokerClient({
  connectNative: (hostName) => chrome.runtime.connectNative(hostName),
});
const sessionRoutes = new SessionRouteRegistry();
const productCommits = new ProductCommitLinearizer();
const lifetimePorts = new Set<chrome.runtime.Port>();
let permissionEpoch = 0;
let registrationTail: Promise<void> = Promise.resolve();

function sendToRoute(
  route: TrustedSessionRoute,
  message: ContentControlMessage | ProductControlMessage,
): Promise<unknown> {
  return chrome.tabs.sendMessage(route.tabId, message, {
    frameId: route.frameId,
    documentId: route.documentId,
  });
}

function disablePortDocument(port: chrome.runtime.Port): Promise<unknown> {
  const sender = port.sender;
  if (
    sender === undefined ||
    sender.tab?.id === undefined ||
    sender.frameId !== 0 ||
    typeof sender.documentId !== "string" ||
    sender.documentId.length === 0 ||
    sender.origin !== DILLINGER_ORIGIN ||
    sender.url !== DILLINGER_URL
  ) {
    return Promise.resolve(undefined);
  }
  const tabId = sender.tab.id;
  return chrome.tabs.sendMessage(
    tabId,
    { kind: "badi.product.disable.v1" } satisfies ProductControlMessage,
    { frameId: 0, documentId: sender.documentId },
  );
}

async function retireRoute(sessionId: string, route: TrustedSessionRoute): Promise<void> {
  if (!sessionRoutes.delete(sessionId, route)) return;
  productCommits.revokeSession(sessionId);
  await broker.closeSession(sessionId).catch(() => undefined);
}

async function retireAll(disable: boolean): Promise<void> {
  productCommits.revokeAll();
  const entries = sessionRoutes.entries();
  for (const { sessionId, route } of entries) sessionRoutes.delete(sessionId, route);
  if (disable) {
    await Promise.allSettled(
      entries.map(({ route }) => sendToRoute(route, { kind: "badi.product.disable.v1" })),
    );
  }
  await Promise.allSettled(entries.map(({ sessionId }) => broker.closeSession(sessionId)));
}

async function performProductScriptReconciliation(): Promise<void> {
  const granted = await hasDillingerAccess();
  const registered = await chrome.scripting.getRegisteredContentScripts({
    ids: [PRODUCT_SCRIPT_ID],
  });
  if (granted && registered.length === 0) {
    await chrome.scripting.registerContentScripts([
      {
        id: PRODUCT_SCRIPT_ID,
        matches: [DILLINGER_CONTENT_MATCH],
        js: ["product-content-script.js"],
        allFrames: false,
        runAt: "document_idle",
        persistAcrossSessions: false,
        world: "ISOLATED",
      },
    ]);
  } else if (!granted && registered.length > 0) {
    await chrome.scripting.unregisterContentScripts({ ids: [PRODUCT_SCRIPT_ID] });
  }
}

function reconcileProductScriptRegistration(): Promise<void> {
  const next = registrationTail
    .catch(() => undefined)
    .then(() => performProductScriptReconciliation());
  registrationTail = next.catch(() => undefined);
  return next;
}

async function exactActiveDillingerTab(): Promise<chrome.tabs.Tab | null> {
  const tabs = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  if (tabs.length !== 1) return null;
  const tab = tabs[0];
  if (
    tab === undefined ||
    typeof tab.id !== "number" ||
    typeof tab.windowId !== "number" ||
    tab.active !== true ||
    tab.incognito !== false ||
    tab.discarded !== false ||
    tab.frozen !== false ||
    !isExactDillingerUrl(tab.url)
  ) {
    return null;
  }
  const window = await chrome.windows.get(tab.windowId);
  return window.focused === true ? tab : null;
}

async function routeIsCurrentlyAuthorized(route: TrustedSessionRoute): Promise<boolean> {
  if (route.frameId !== 0 || route.origin !== DILLINGER_ORIGIN) return false;
  const epoch = permissionEpoch;
  const [granted, tab] = await Promise.all([
    hasDillingerAccess(),
    chrome.tabs.get(route.tabId).catch(() => null),
  ]);
  if (!granted || epoch !== permissionEpoch || tab === null || typeof tab.windowId !== "number") {
    return false;
  }
  const window = await chrome.windows.get(tab.windowId).catch(() => null);
  return (
    epoch === permissionEpoch &&
    window?.focused === true &&
    tab.active === true &&
    tab.incognito === false &&
    tab.discarded === false &&
    tab.frozen === false &&
    tab.url === DILLINGER_URL
  );
}

async function executeSnapshot(route: TrustedSessionRoute): Promise<MonacoSnapshot | null> {
  if (!(await routeIsCurrentlyAuthorized(route))) return null;
  const results = await chrome.scripting.executeScript({
    target: { tabId: route.tabId, documentIds: [route.documentId] },
    world: "MAIN",
    func: readDillingerMonacoSnapshotInMainWorld,
  });
  if (
    results.length !== 1 ||
    results[0]?.frameId !== 0 ||
    results[0]?.documentId !== route.documentId ||
    !(await routeIsCurrentlyAuthorized(route))
  ) {
    return null;
  }
  return parseSnapshotReply({ ok: true, snapshot: results[0].result });
}

async function executeApply(
  route: TrustedSessionRoute,
  command: Extract<ProductBridgeCommand, { kind: "badi.product.monaco.apply.v1" }>,
): Promise<boolean> {
  const { authorization } = command;
  if (
    authorization.acceptance !== "all" ||
    sanitizeSuggestion(authorization.text) !== authorization.text
  ) {
    return false;
  }
  const expectedEpoch = productCommits.epoch;
  if (!(await routeIsCurrentlyAuthorized(route))) return false;
  const execution = productCommits.invoke(
    authorization,
    route,
    expectedEpoch,
    () =>
      chrome.scripting.executeScript({
        target: { tabId: route.tabId, documentIds: [route.documentId] },
        world: "MAIN",
        func: applyDillingerMonacoEditInMainWorld,
        args: [command.expected, authorization.text],
      }),
  );
  if (execution === null) return false;
  const results = await execution;
  return (
    results.length === 1 &&
    results[0]?.frameId === 0 &&
    results[0]?.documentId === route.documentId &&
    results[0]?.result === true
  );
}

async function handleBridge(
  command: ProductBridgeCommand,
  sender: chrome.runtime.MessageSender,
): Promise<ProductBridgeReply> {
  if (!sessionRoutes.matches(command.sessionId, sender)) {
    return { ok: false, error: "Dillinger session is bound to another document" };
  }
  const route = sessionRoutes.get(command.sessionId);
  if (route === null) return { ok: false, error: "Dillinger route is unavailable" };
  if (command.kind === "badi.product.monaco.snapshot.v1") {
    return { ok: true, snapshot: await executeSnapshot(route) };
  }
  return { ok: true, applied: await executeApply(route, command) };
}

async function handleBroker(
  command: RuntimeCommand,
  sender: chrome.runtime.MessageSender,
): Promise<RuntimeReply> {
  const sessionId = commandSessionId(command);
  if (!sessionRoutes.matches(sessionId, sender)) {
    return { ok: false, error: "Dillinger session is bound to another document" };
  }
  const route = sessionRoutes.get(sessionId);
  if (route === null || !(await routeIsCurrentlyAuthorized(route))) {
    return { ok: false, error: "Dillinger route is no longer active or authorized" };
  }
  switch (command.kind) {
    case "badi.bootstrap.v1": {
      productCommits.revokeSession(command.sessionId);
      const bootstrap = await broker.bootstrap(command.sessionId, DILLINGER_ORIGIN);
      if (!sessionRoutes.matches(command.sessionId, sender)) {
        return { ok: false, error: "Dillinger bootstrap route was displaced" };
      }
      return { ok: true, paused: bootstrap.paused, policy: bootstrap.policy };
    }
    case "badi.suggest.v1":
      productCommits.revokeSession(command.request.sessionId);
      if (command.request.origin !== DILLINGER_ORIGIN) {
        return { ok: false, error: "Suggestion origin does not match Dillinger" };
      }
      return { ok: true, response: await broker.requestSuggestion(command.request) };
    case "badi.cancel.v1":
      productCommits.revokeSession(command.request.sessionId);
      await broker.cancelSuggestion(command.request);
      return { ok: true };
    case "badi.session.close.v1":
      productCommits.revokeSession(command.sessionId);
      await broker.closeSession(command.sessionId);
      return { ok: true };
    case "badi.dismiss.v1":
      productCommits.revokeSession(command.address.sessionId);
      await broker.dismissSuggestion(command.address);
      return { ok: true };
    case "badi.commit.authorize.v1": {
      const expectedEpoch = productCommits.epoch;
      const response = await broker.authorizeCommit(command.request);
      if (
        expectedEpoch !== productCommits.epoch ||
        !sessionRoutes.matches(command.request.sessionId, sender) ||
        !(await routeIsCurrentlyAuthorized(route)) ||
        expectedEpoch !== productCommits.epoch
      ) {
        return { ok: false, error: "Dillinger commit authorization became stale" };
      }
      productCommits.issue(response, route);
      return { ok: true, response };
    }
    case "badi.commit.result.v1":
      productCommits.revokeSession(command.notice.sessionId);
      await broker.reportCommit(command.notice);
      return { ok: true };
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

function trustedPopupSender(sender: chrome.runtime.MessageSender): boolean {
  return (
    sender.id === chrome.runtime.id &&
    sender.tab === undefined &&
    sender.url === chrome.runtime.getURL("product-access.html")
  );
}

async function activateCurrentTarget(): Promise<boolean> {
  if (!(await hasDillingerAccess())) return false;
  await reconcileProductScriptRegistration();
  const tab = await exactActiveDillingerTab();
  if (tab?.id === undefined) return true;
  await chrome.scripting.executeScript({
    target: { tabId: tab.id, frameIds: [0] },
    world: "ISOLATED",
    files: ["product-content-script.js"],
  });
  return true;
}

chrome.runtime.onConnect.addListener((port) => {
  if (
    port.name !== PRODUCT_LIFETIME_PORT ||
    port.sender === undefined ||
    !isTrustedDillingerSender(port.sender, chrome.runtime.id)
  ) {
    port.disconnect();
    return;
  }
  const sender = port.sender;
  lifetimePorts.add(port);
  void hasDillingerAccess().then((granted) => {
    if (!granted) port.disconnect();
  });
  port.onDisconnect.addListener(() => {
    lifetimePorts.delete(port);
    const sessionIds = sessionRoutes.deleteDocument(sender);
    for (const sessionId of sessionIds) productCommits.revokeSession(sessionId);
    void Promise.allSettled(sessionIds.map((sessionId) => broker.closeSession(sessionId)));
  });
});

chrome.runtime.onMessage.addListener((message: unknown, sender, sendResponse) => {
  if (isProductExtensionCommand(message)) {
    if (!trustedPopupSender(sender)) {
      sendResponse({ ok: false, error: "Untrusted product access sender" } satisfies ProductExtensionReply);
      return false;
    }
    void (message.kind === "badi.product.activate-current.v1"
      ? activateCurrentTarget()
      : hasDillingerAccess()
    ).then(
      (granted) => sendResponse({ ok: true, granted } satisfies ProductExtensionReply),
      () => sendResponse({ ok: false, error: "Product permission operation failed" } satisfies ProductExtensionReply),
    );
    return true;
  }
  const runtimeCommand = isRuntimeCommand(message);
  const bridgeCommand = isProductBridgeCommand(message);
  if (!runtimeCommand && !bridgeCommand) return false;
  if (!isTrustedDillingerSender(sender, chrome.runtime.id)) {
    sendResponse({ ok: false, error: "Untrusted Dillinger sender" });
    return false;
  }
  let displacedSessionIds: readonly string[] = [];
  if (runtimeCommand && message.kind === "badi.bootstrap.v1") {
    const subscription = sessionRoutes.subscribe(message.sessionId, sender);
    if (subscription === null) {
      sendResponse({ ok: false, error: "Dillinger session cannot migrate documents" });
      return false;
    }
    displacedSessionIds = subscription.displacedSessionIds;
  }
  void (async (): Promise<RuntimeReply | ProductBridgeReply> => {
    for (const sessionId of displacedSessionIds) productCommits.revokeSession(sessionId);
    await Promise.allSettled(
      displacedSessionIds.map((sessionId) => broker.closeSession(sessionId)),
    );
    return runtimeCommand
      ? handleBroker(message, sender)
      : handleBridge(message, sender);
  })().then(
    (reply) => sendResponse(reply),
    () => sendResponse({ ok: false, error: "Dillinger adapter operation failed" }),
  );
  return true;
});

broker.setCommitRevocationHandler((request) => {
  productCommits.revokeSession(request.sessionId);
  const route = sessionRoutes.get(request.sessionId);
  if (route === null) return;
  void sendToRoute(route, {
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
  }).catch(() => undefined);
});

broker.setSuggestionClearHandler((event) => {
  productCommits.revokeSession(event.sessionId);
  const route = sessionRoutes.get(event.sessionId);
  if (route !== null) {
    void sendToRoute(route, { kind: "badi.suggestion.clear.v1", event }).catch(
      () => undefined,
    );
  }
});

broker.setDisconnectHandler(() => {
  productCommits.revokeAll();
  void Promise.allSettled(
    sessionRoutes.snapshot().map((route) =>
      sendToRoute(route, { kind: "badi.transport.disconnected.v1" }),
    ),
  );
});

broker.setAuthorityChangedHandler(async () => {
  productCommits.revokeAll();
  const entries = sessionRoutes.entries();
  await Promise.allSettled(
    entries.map(({ route }) => sendToRoute(route, { kind: "badi.control.v1", action: "pause" })),
  );
  for (const { sessionId, route } of entries) await retireRoute(sessionId, route);
});

chrome.permissions.onAdded.addListener((permissions) => {
  if (!permissionSetIncludesDillinger(permissions)) return;
  permissionEpoch += 1;
  productCommits.revokeAll();
  void reconcileProductScriptRegistration().catch(() => undefined);
});

chrome.permissions.onRemoved.addListener((permissions) => {
  if (!permissionSetIncludesDillinger(permissions)) return;
  permissionEpoch += 1;
  productCommits.revokeAll();
  const ports = [...lifetimePorts];
  lifetimePorts.clear();
  void Promise.allSettled([
    ...ports.map((port) => disablePortDocument(port)),
    retireAll(true),
  ])
    .finally(() => {
      for (const port of ports) port.disconnect();
      return reconcileProductScriptRegistration();
    })
    .catch(() => undefined);
});

chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.status !== "loading" && changeInfo.url === undefined) return;
  const sessionIds = sessionRoutes.deleteTab(tabId);
  for (const sessionId of sessionIds) productCommits.revokeSession(sessionId);
  void Promise.allSettled(sessionIds.map((sessionId) => broker.closeSession(sessionId)));
});

chrome.tabs.onRemoved.addListener((tabId) => {
  const sessionIds = sessionRoutes.deleteTab(tabId);
  for (const sessionId of sessionIds) productCommits.revokeSession(sessionId);
  void Promise.allSettled(sessionIds.map((sessionId) => broker.closeSession(sessionId)));
});

chrome.tabs.onActivated.addListener(({ tabId }) => {
  const retired = sessionRoutes.entries().filter(({ route }) => route.tabId !== tabId);
  for (const { sessionId, route } of retired) {
    sessionRoutes.delete(sessionId, route);
    productCommits.revokeSession(sessionId);
  }
  void Promise.allSettled(retired.map(({ sessionId }) => broker.closeSession(sessionId)));
});

chrome.commands.onCommand.addListener((command, tab) => {
  if (command !== "accept-dillinger-suggestion" || tab?.id === undefined) return;
  void (async () => {
    if (!(await hasDillingerAccess())) return;
    const current = await exactActiveDillingerTab();
    if (current?.id !== tab.id) return;
    const entries = sessionRoutes.entries().filter(({ route }) => route.tabId === tab.id);
    if (entries.length !== 1) return;
    const entry = entries[0];
    if (entry === undefined || !(await routeIsCurrentlyAuthorized(entry.route))) return;
    await sendToRoute(entry.route, { kind: "badi.product.accept-all.v1" });
  })().catch(() => undefined);
});

void reconcileProductScriptRegistration().catch(() => undefined);
