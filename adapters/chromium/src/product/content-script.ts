import type { BootstrapState, TargetPolicy } from "../shared/model";
import { isContentControlMessage } from "../shared/runtime-messages";
import { RuntimeSuggestionTransport } from "../content/runtime-transport";
import { MonacoController } from "./monaco-controller";
import { RuntimeMonacoBridge } from "./monaco-runtime-bridge";
import { isProductControlMessage } from "./messages";
import {
  DILLINGER_ORIGIN,
  PRODUCT_LIFETIME_PORT,
  isExactDillingerDocument,
} from "./target";

const INSTALLATION_KEY = "__BADI_DILLINGER_PRODUCT_V1__";
const isolatedGlobal = globalThis as typeof globalThis & Record<string, unknown>;

if (isolatedGlobal[INSTALLATION_KEY] !== true && isExactDillingerDocument(document)) {
  isolatedGlobal[INSTALLATION_KEY] = true;
  const transport = new RuntimeSuggestionTransport();
  const bridge = new RuntimeMonacoBridge();
  const sessionId = crypto.randomUUID();
  let controller: MonacoController | null = null;
  let lifetimePort: chrome.runtime.Port | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectAttempts = 0;
  let bootstrapping = false;
  let bootstrapGeneration = 0;
  let disabled = false;
  let currentPolicy: TargetPolicy | null = null;
  let navigation: EventTarget | null = null;
  const MAX_RECONNECT_ATTEMPTS = 5;

  const policyAllowsProduct = (state: BootstrapState | null = null): boolean => {
    const policy = state?.policy ?? currentPolicy;
    return (
      !disabled &&
      policy !== null &&
      policy.paused === false &&
      policy.activation === "always" &&
      policy.contextAllowed &&
      policy.displayAllowed &&
      policy.suggestionsAllowed
    );
  };

  const disposeController = (invalidateTransport = false): void => {
    if (controller === null) return;
    controller.dispose(invalidateTransport);
    controller = null;
  };

  const applyPolicy = (policy: TargetPolicy): void => {
    if (currentPolicy !== null && policy.authorityEpoch < currentPolicy.authorityEpoch) return;
    currentPolicy = policy;
    if (!policyAllowsProduct()) {
      disposeController();
      return;
    }
    if (controller === null) {
      controller = new MonacoController({
        transport,
        bridge,
        sessionId,
        origin: DILLINGER_ORIGIN,
        isCurrentDocument: () => isExactDillingerDocument(document),
      });
      controller.start();
    }
    controller.resume();
  };

  const fence = (invalidateTransport: boolean): void => {
    bootstrapGeneration += 1;
    bootstrapping = false;
    currentPolicy = null;
    disposeController(invalidateTransport);
  };

  const stopPermanently = (): void => {
    if (disabled) return;
    disabled = true;
    fence(true);
    if (reconnectTimer !== null) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    lifetimePort?.disconnect();
    lifetimePort = null;
    chrome.runtime.onMessage.removeListener(onRuntimeMessage);
    window.removeEventListener("focus", recover, true);
    window.removeEventListener("pageshow", recover, true);
    window.removeEventListener("popstate", recover, true);
    window.removeEventListener("hashchange", recover, true);
    window.removeEventListener("blur", recover, true);
    window.removeEventListener("pagehide", stopPermanently, true);
    document.removeEventListener("visibilitychange", recover, true);
    navigation?.removeEventListener("currententrychange", recover);
    navigation = null;
    isolatedGlobal[INSTALLATION_KEY] = false;
  };

  const documentCanBootstrap = (): boolean =>
    !disabled &&
    isExactDillingerDocument(document) &&
    document.visibilityState === "visible" &&
    document.hasFocus();

  const bootstrap = (): void => {
    if (bootstrapping || !documentCanBootstrap()) return;
    bootstrapping = true;
    const generation = ++bootstrapGeneration;
    void transport.bootstrap(sessionId).then(
      (state) => {
        if (disabled || generation !== bootstrapGeneration) return;
        bootstrapping = false;
        reconnectAttempts = 0;
        applyPolicy(state.policy);
        reconnectLifetime();
      },
      () => {
        if (generation === bootstrapGeneration) bootstrapping = false;
      },
    );
  };

  const reconnectLifetime = (): void => {
    if (
      lifetimePort !== null ||
      reconnectTimer !== null ||
      reconnectAttempts >= MAX_RECONNECT_ATTEMPTS ||
      !documentCanBootstrap()
    ) {
      return;
    }
    reconnectAttempts += 1;
    const port = chrome.runtime.connect({ name: PRODUCT_LIFETIME_PORT });
    lifetimePort = port;
    port.onDisconnect.addListener(() => {
      if (lifetimePort !== port) return;
      lifetimePort = null;
      fence(true);
      if (disabled || reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) return;
      const delayMs = Math.min(2_000, 100 * 2 ** (reconnectAttempts - 1));
      reconnectTimer = setTimeout(() => {
        reconnectTimer = null;
        bootstrap();
        reconnectLifetime();
      }, delayMs);
    });
  };

  function onRuntimeMessage(
    message: unknown,
    sender: chrome.runtime.MessageSender,
    sendResponse: (response?: unknown) => void,
  ): boolean {
    if (sender.id !== chrome.runtime.id) return false;
    const respond = typeof sendResponse === "function" ? sendResponse : () => undefined;
    if (isProductControlMessage(message)) {
      if (message.kind === "badi.product.disable.v1") {
        stopPermanently();
        respond({ applied: true });
      } else if (message.kind === "badi.product.worker-restarted.v1") {
        fence(true);
        bootstrap();
        respond({ applied: true });
      } else {
        controller?.acceptAll();
        respond({ applied: controller?.suggestionVisible === true });
      }
      return false;
    }
    if (!isContentControlMessage(message) || disabled) return false;
    if (message.kind === "badi.policy.v1") {
      applyPolicy(message.policy);
      respond({ applied: true, authorityEpoch: currentPolicy?.authorityEpoch ?? -1 });
      return false;
    }
    if (message.kind === "badi.transport.disconnected.v1") {
      fence(true);
      bootstrap();
      return false;
    }
    if (message.kind === "badi.commit.revoke.v1") {
      controller?.revokeCommit(message.address);
      return false;
    }
    if (message.kind === "badi.suggestion.clear.v1") {
      controller?.clearFromBroker(message.event);
      return false;
    }
    switch (message.action) {
      case "pause":
        fence(false);
        respond({ applied: true, paused: true });
        break;
      case "resume":
        // Only a versioned policy or fresh bootstrap may clear the fence.
        respond({ applied: true, paused: true });
        break;
      case "accept_all":
        controller?.acceptAll();
        break;
      case "dismiss":
        controller?.dismiss();
        break;
      case "accept_word":
        // The product proof exposes one target-native transaction only.
        break;
    }
    return false;
  }

  function recover(): void {
    if (!isExactDillingerDocument(document)) {
      stopPermanently();
      return;
    }
    if (!documentCanBootstrap()) {
      controller?.pause();
      return;
    }
    if (currentPolicy !== null && policyAllowsProduct()) controller?.resume();
    bootstrap();
    reconnectLifetime();
  }

  chrome.runtime.onMessage.addListener(onRuntimeMessage);
  window.addEventListener("focus", recover, true);
  window.addEventListener("pageshow", recover, true);
  window.addEventListener("popstate", recover, true);
  window.addEventListener("hashchange", recover, true);
  window.addEventListener("blur", recover, true);
  window.addEventListener("pagehide", stopPermanently, true);
  document.addEventListener("visibilitychange", recover, true);
  navigation = (window as Window & { navigation?: EventTarget }).navigation ?? null;
  navigation?.addEventListener("currententrychange", recover);

  reconnectLifetime();
  bootstrap();
}
