import { isContentControlMessage } from "../shared/runtime-messages";
import { isExpectedFixtureDocument } from "../shared/fixture-document";
import { FieldController } from "./field-controller";
import { RuntimeSuggestionTransport } from "./runtime-transport";

if (isExpectedFixtureDocument(document)) {
  const transport = new RuntimeSuggestionTransport();
  // The route and every later field request share this document-scoped ID.
  const sessionId = crypto.randomUUID();
  let controller: FieldController | null = null;
  let queuedPaused: boolean | null = null;
  let bootstrapping = false;
  let bootstrapAttempts = 0;
  let completedBootstrapPaused: boolean | null = null;
  const MAX_BOOTSTRAP_ATTEMPTS = 3;

  chrome.runtime.onMessage.addListener((message: unknown) => {
    if (!isContentControlMessage(message)) {
      return;
    }
    if (controller === null) {
      if (message.kind === "badi.control.v1" && message.action === "pause") {
        queuedPaused = true;
      } else if (message.kind === "badi.control.v1" && message.action === "resume") {
        queuedPaused = false;
      }
      return;
    }
    if (message.kind === "badi.commit.revoke.v1") {
      controller.revokeCommit(message.address);
      return;
    }
    if (message.kind === "badi.suggestion.clear.v1") {
      controller.clearFromBroker(message.event);
      return;
    }
    if (message.kind === "badi.transport.disconnected.v1") {
      controller.invalidateTransport();
      return;
    }
    switch (message.action) {
      case "pause":
        controller.pause();
        break;
      case "resume":
        controller.resume();
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
      completedBootstrapPaused === null ||
      !isExpectedFixtureDocument(document)
    ) {
      return;
    }
    const bootstrapPaused = completedBootstrapPaused;
    completedBootstrapPaused = null;
    controller = new FieldController({
      transport,
      sessionId,
      isCurrentDocument: () => isExpectedFixtureDocument(document),
    });
    if (queuedPaused ?? bootstrapPaused) controller.pause();
    controller.start();
    removeRecoveryListeners();
  };

  const tryBootstrap = (): void => {
    if (
      controller !== null ||
      bootstrapping ||
      bootstrapAttempts >= MAX_BOOTSTRAP_ATTEMPTS ||
      !isExpectedFixtureDocument(document)
    ) {
      return;
    }
    bootstrapping = true;
    bootstrapAttempts += 1;
    void transport.bootstrap(sessionId).then(
      (bootstrapPaused) => {
        bootstrapping = false;
        completedBootstrapPaused = bootstrapPaused;
        startControllerFromBootstrap();
      },
      () => {
        bootstrapping = false;
        if (bootstrapAttempts >= MAX_BOOTSTRAP_ATTEMPTS) removeRecoveryListeners();
      },
    );
  };

  function recoverBootstrap(): void {
    // Only the initial, content-free registration may run in a background tab.
    // Later attempts are bounded and require a foreground exact document.
    if (
      document.visibilityState !== "visible" ||
      !document.hasFocus() ||
      !isExpectedFixtureDocument(document)
    ) {
      return;
    }
    startControllerFromBootstrap();
    tryBootstrap();
  }

  window.addEventListener("focus", recoverBootstrap, true);
  window.addEventListener("pageshow", recoverBootstrap, true);
  window.addEventListener("popstate", recoverBootstrap, true);
  window.addEventListener("hashchange", recoverBootstrap, true);
  document.addEventListener("visibilitychange", recoverBootstrap, true);
  navigation?.addEventListener("currententrychange", recoverBootstrap);
  tryBootstrap();
}
