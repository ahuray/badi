import { isContentControlMessage } from "../shared/runtime-messages";
import { FieldController } from "./field-controller";
import { RuntimeSuggestionTransport } from "./runtime-transport";
import type { BootstrapState, TargetPolicy } from "../shared/model";

export function startBrowserContent(isCurrentDocument: (document: Document) => boolean, readingLeaseMs = 600): (() => void) | undefined {
  if (!isCurrentDocument(document)) return;
  const transport = new RuntimeSuggestionTransport();
  const sessionId = crypto.randomUUID();
  let controller: FieldController | null = null;
  let currentPaused = true;
  let currentPolicy: TargetPolicy | null = null;
  let bootstrapping = false;
  let bootstrapAttempts = 0;
  let bootstrapRetryTimer: ReturnType<typeof setTimeout> | null = null;
  let bootstrapCompleted = false;
  let bootstrapGeneration = 0;
  let completedBootstrap: BootstrapState | null = null;
  let authorityFenced = false;
  let lifetimePort: chrome.runtime.Port | null = null;
  let lifetimeReconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let lifetimeReconnectAttempts = 0;
  const MAX_BOOTSTRAP_ATTEMPTS = readingLeaseMs > 600 ? 10 : 3;
  const MAX_LIFETIME_RECONNECT_ATTEMPTS = 5;

  const policyAllowsController = (): boolean =>
    !authorityFenced &&
    !currentPaused &&
    currentPolicy?.paused === false &&
    currentPolicy.activation === "always" &&
    currentPolicy.contextAllowed &&
    currentPolicy.displayAllowed &&
    currentPolicy.suggestionsAllowed;

  const disposeController = (invalidateTransport = false): void => {
    if (controller === null) return;
    controller.pause();
    if (invalidateTransport) controller.invalidateTransport();
    controller.dispose();
    controller = null;
  };

  const applyAvailability = (): void => {
    if (controller === null) return;
    if (policyAllowsController()) {
      controller.resume();
      return;
    }
    disposeController();
    installRecoveryListeners();
  };

  const fenceAuthority = (clearPolicy: boolean, invalidateTransport = false): void => {
    if (bootstrapRetryTimer !== null) clearTimeout(bootstrapRetryTimer);
    bootstrapRetryTimer = null;
    bootstrapGeneration += 1;
    bootstrapCompleted = false;
    completedBootstrap = null;
    bootstrapping = false;
    if (!invalidateTransport) bootstrapAttempts = 0;
    authorityFenced = true;
    currentPaused = true;
    if (clearPolicy) currentPolicy = null;
    disposeController(invalidateTransport);
    installRecoveryListeners();
  };

  chrome.runtime.onMessage.addListener((message: unknown, _sender, sendResponse) => {
    const respond = typeof sendResponse === "function" ? sendResponse : () => undefined;
    if (!isContentControlMessage(message)) {
      return false;
    }
    if (message.kind === "badi.policy.v1") {
      const accepted =
        currentPolicy === null ||
        message.policy.authorityEpoch >= currentPolicy.authorityEpoch;
      if (accepted) {
        currentPolicy = message.policy;
        currentPaused = message.policy.paused;
        authorityFenced = false;
        bootstrapCompleted = true;
        completedBootstrap = null;
        applyAvailability();
        startControllerFromBootstrap();
      }
      respond({
        applied: accepted,
        authorityEpoch: currentPolicy?.authorityEpoch ?? -1,
      });
      return false;
    }
    if (message.kind === "badi.control.v1" && message.action === "pause") {
      // A pause is the first half of an authority refresh. Invalidate every
      // older bootstrap result and wait for the versioned policy that follows.
      fenceAuthority(false);
      respond({ applied: true, paused: true });
      return false;
    }
    if (message.kind === "badi.control.v1" && message.action === "resume") {
      // An unversioned control message cannot clear an authority fence.
      if (!authorityFenced) {
        currentPaused = false;
        applyAvailability();
        startControllerFromBootstrap();
      }
      respond({ applied: true, paused: !policyAllowsController() });
      return false;
    }
    if (message.kind === "badi.transport.disconnected.v1") {
      // Failed reconnects also broadcast disconnect. Once authority is gone,
      // retain backoff instead of exhausting every attempt before a cold model
      // is ready. Still fence late bootstrap replies from the disconnected host.
      const retrying = readingLeaseMs > 600 && authorityFenced &&
        (bootstrapping || bootstrapRetryTimer !== null);
      fenceAuthority(true, true);
      if (retrying) scheduleBootstrapRetry();
      else tryBootstrap();
      return false;
    }
    if (controller === null) return false;
    if (message.kind === "badi.commit.revoke.v1") {
      controller.revokeCommit(message.address);
      return false;
    }
    if (message.kind === "badi.suggestion.clear.v1") {
      controller.clearFromBroker(message.event);
      return false;
    }
    switch (message.action) {
      case "pause":
        currentPaused = true;
        applyAvailability();
        break;
      case "resume":
        currentPaused = false;
        applyAvailability();
        break;
      case "accept_word":
        controller.acceptWord();
        break;
      case "accept_all":
        controller.acceptAll();
        break;
      case "dismiss":
        controller.dismiss();
        break;
    }
    return false;
  });

  const navigation = (window as Window & { navigation?: EventTarget }).navigation ?? null;

  const removeRecoveryListeners = (): void => {
    window.removeEventListener("focus", recoverBootstrap, true);
    window.removeEventListener("pageshow", recoverBootstrap, true);
    window.removeEventListener("popstate", recoverBootstrap, true);
    window.removeEventListener("hashchange", recoverBootstrap, true);
    document.removeEventListener("visibilitychange", recoverBootstrap, true);
    navigation?.removeEventListener("currententrychange", recoverBootstrap);
  };

  const startControllerFromBootstrap = (): void => {
    if (
      controller !== null ||
      (!bootstrapCompleted && completedBootstrap === null) ||
      !isCurrentDocument(document)
    ) {
      return;
    }
    if (completedBootstrap !== null) {
      const bootstrap = completedBootstrap;
      completedBootstrap = null;
      bootstrapCompleted = true;
      if (
        currentPolicy === null ||
        bootstrap.policy.authorityEpoch >= currentPolicy.authorityEpoch
      ) {
        currentPolicy = bootstrap.policy;
        currentPaused = bootstrap.paused;
        authorityFenced = false;
      }
    }
    if (!policyAllowsController()) return;
    controller = new FieldController({
      transport,
      sessionId,
      readingLeaseMs,
      // The exact fixture keeps its latency contract; a real local model may
      // need longer under load. Revision/focus checks still cancel stale work.
      generationMaxAgeMs: readingLeaseMs > 600 ? 2000 : 600,
      requestOnTab: readingLeaseMs > 600,
      tabAcceptsWord: readingLeaseMs > 600,
      preserveUndo: readingLeaseMs > 600,
      fallbackLanguage: readingLeaseMs > 600 ? navigator.language : undefined,
      isCurrentDocument: () => isCurrentDocument(document),
    });
    applyAvailability();
    controller.start();
    removeRecoveryListeners();
  };

  const tryBootstrap = (): void => {
    if (
      controller !== null ||
      bootstrapping ||
      bootstrapAttempts >= MAX_BOOTSTRAP_ATTEMPTS ||
      !isCurrentDocument(document)
    ) {
      return;
    }
    bootstrapping = true;
    bootstrapAttempts += 1;
    const generation = bootstrapGeneration;
    void transport.bootstrap(sessionId).then(
      (bootstrap) => {
        if (generation !== bootstrapGeneration) return;
        bootstrapping = false;
        if (bootstrapRetryTimer !== null) clearTimeout(bootstrapRetryTimer);
        bootstrapRetryTimer = null;
        bootstrapAttempts = 0;
        lifetimeReconnectAttempts = 0;
        if (lifetimeReconnectTimer !== null) {
          clearTimeout(lifetimeReconnectTimer);
          lifetimeReconnectTimer = null;
        }
        reconnectPolicyLifetime();
        completedBootstrap = bootstrap;
        startControllerFromBootstrap();
      },
      (error: unknown) => {
        console.warn("Badi bootstrap failed", String(error));
        if (generation !== bootstrapGeneration) return;
        bootstrapping = false;
        scheduleBootstrapRetry();
        if (readingLeaseMs <= 600 && bootstrapAttempts >= MAX_BOOTSTRAP_ATTEMPTS) removeRecoveryListeners();
      },
    );
  };

  function scheduleBootstrapRetry(): void {
    if (readingLeaseMs <= 600 || bootstrapAttempts >= MAX_BOOTSTRAP_ATTEMPTS) return;
    const generation = bootstrapGeneration;
    if (bootstrapRetryTimer !== null) clearTimeout(bootstrapRetryTimer);
    bootstrapRetryTimer = setTimeout(() => {
      bootstrapRetryTimer = null;
      if (generation === bootstrapGeneration && document.visibilityState === "visible" && document.hasFocus()) {
        tryBootstrap();
      }
    }, Math.min(5000, 250 * 2 ** (bootstrapAttempts - 1)));
  }

  function recoverBootstrap(): void {
    // Only the initial, content-free registration may run in a background tab.
    // Later attempts are bounded and require a foreground exact document.
    if (
      document.visibilityState !== "visible" ||
      !document.hasFocus() ||
      !isCurrentDocument(document)
    ) {
      return;
    }
    if (readingLeaseMs > 600 && bootstrapAttempts >= MAX_BOOTSTRAP_ATTEMPTS) bootstrapAttempts = 0;
    reconnectPolicyLifetime();
    startControllerFromBootstrap();
    tryBootstrap();
  }

  const installRecoveryListeners = (): void => {
    window.addEventListener("focus", recoverBootstrap, true);
    window.addEventListener("pageshow", recoverBootstrap, true);
    window.addEventListener("popstate", recoverBootstrap, true);
    window.addEventListener("hashchange", recoverBootstrap, true);
    document.addEventListener("visibilitychange", recoverBootstrap, true);
    navigation?.addEventListener("currententrychange", recoverBootstrap);
  };

  const reconnectPolicyLifetime = (): void => {
    if (
      lifetimePort !== null ||
      lifetimeReconnectTimer !== null ||
      lifetimeReconnectAttempts >= MAX_LIFETIME_RECONNECT_ATTEMPTS ||
      !isCurrentDocument(document)
    ) {
      return;
    }
    lifetimeReconnectAttempts += 1;
    const port = chrome.runtime.connect({ name: "badi-policy-lifetime-v1" });
    lifetimePort = port;
    port.onDisconnect.addListener(() => {
      if (lifetimePort !== port) return;
      lifetimePort = null;
      fenceAuthority(true, true);
      tryBootstrap();
      if (
        lifetimeReconnectAttempts < MAX_LIFETIME_RECONNECT_ATTEMPTS &&
        isCurrentDocument(document)
      ) {
        const delayMs = Math.min(2_000, 100 * 2 ** (lifetimeReconnectAttempts - 1));
        lifetimeReconnectTimer = setTimeout(() => {
          lifetimeReconnectTimer = null;
          reconnectPolicyLifetime();
        }, delayMs);
      }
    });
  };

  installRecoveryListeners();
  reconnectPolicyLifetime();
  tryBootstrap();
  return () => {
    fenceAuthority(true);
    lifetimeReconnectAttempts = 0;
    recoverBootstrap();
  };
}
