import { startBrowserWorker } from "./browser-worker";
import { isTrustedFixtureBootstrapSender, isTrustedFixtureSender, isFocusedFixtureTab } from "./fixture-boundary";

startBrowserWorker({ bootstrap: isTrustedFixtureBootstrapSender,
  sender: isTrustedFixtureSender, focused: isFocusedFixtureTab });
