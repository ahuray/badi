import { isContentControlMessage } from "../shared/runtime-messages";
import { FieldController } from "./field-controller";
import { RuntimeSuggestionTransport } from "./runtime-transport";

const EXPECTED_FIXTURE_ORIGIN = "http://localhost:4173";
const EXPECTED_FIXTURE_PATH = "/chromium.html";
const EXPECTED_FIXTURE_URL = `${EXPECTED_FIXTURE_ORIGIN}${EXPECTED_FIXTURE_PATH}`;

if (
  window.top === window &&
  window.location.origin === EXPECTED_FIXTURE_ORIGIN &&
  window.location.pathname === EXPECTED_FIXTURE_PATH &&
  window.location.href === EXPECTED_FIXTURE_URL
) {
  const controller = new FieldController({
    transport: new RuntimeSuggestionTransport(),
  });
  controller.start();

  chrome.runtime.onMessage.addListener((message: unknown) => {
    if (!isContentControlMessage(message)) {
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
}
