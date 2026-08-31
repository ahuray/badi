import {
  hasDillingerAccess,
  removeDillingerAccess,
  requestDillingerAccess,
} from "./permissions";
import type { ProductExtensionCommand, ProductExtensionReply } from "./messages";

const status = document.querySelector<HTMLElement>("#status");
const enable = document.querySelector<HTMLButtonElement>("#enable");
const disable = document.querySelector<HTMLButtonElement>("#disable");

function setStatus(message: string): void {
  if (status !== null) status.textContent = message;
}

async function sync(): Promise<void> {
  const granted = await hasDillingerAccess();
  if (enable !== null) enable.hidden = granted;
  if (disable !== null) disable.hidden = !granted;
  setStatus(granted ? "Dillinger access is enabled." : "Dillinger access is disabled.");
}

enable?.addEventListener("click", () => {
  // Keep request() directly in the click handler so Chrome sees a user gesture.
  const request = requestDillingerAccess();
  void request.then(async (granted) => {
    if (!granted) {
      setStatus("Dillinger access was not granted.");
      return;
    }
    const reply = (await chrome.runtime.sendMessage({
      kind: "badi.product.activate-current.v1",
    } satisfies ProductExtensionCommand)) as ProductExtensionReply;
    if (!reply.ok || !reply.granted) throw new Error("Dillinger activation failed");
    await sync();
  }).catch(() => setStatus("Dillinger access could not be enabled."));
});

disable?.addEventListener("click", () => {
  void removeDillingerAccess().then(async () => {
    await sync();
  }).catch(() => setStatus("Dillinger access could not be removed."));
});

void sync().catch(() => setStatus("Dillinger access state is unavailable."));
