import { hostPattern, permitted, webOrigin } from './boundary';
import { NativeBrokerClient } from '../background/native-client';

const status = document.querySelector<HTMLElement>('#status')!;
const site = document.querySelector<HTMLElement>('#site')!;
const command = document.querySelector<HTMLElement>('#command')!;
const enable = document.querySelector<HTMLButtonElement>('#enable')!;
const disable = document.querySelector<HTMLButtonElement>('#disable')!;
const retry = document.querySelector<HTMLButtonElement>('#retry')!;
const broker = new NativeBrokerClient({ connectNative: name => chrome.runtime.connectNative(name) }, { protocolVersion: 2 });
let origin: string | null = null;
let tabId: number | undefined;
let busy = false;

function working(value: boolean): void {
  busy = value;
  enable.disabled = value || origin === null;
  disable.disabled = value;
  retry.disabled = value;
}

function failure(message: string): void {
  status.textContent = message;
  retry.hidden = origin === null;
}

async function activate(): Promise<void> {
  if (tabId === undefined || origin === null) return;
  const tab = await chrome.tabs.get(tabId);
  if (!tab.active || tab.incognito || webOrigin(tab.url ?? '') !== origin)
    throw new Error('The page changed. Reopen Badi on the site you want to enable.');
  await chrome.scripting.executeScript({ target: { tabId, frameIds: [0] }, files: ['web-content-script.js'] });
}

async function sync(): Promise<void> {
  if (!origin) return;
  const granted = await permitted(origin);
  enable.hidden = granted; disable.hidden = !granted;
  command.textContent = `badi site ${origin} on`;
  command.hidden = false;
  retry.hidden = !granted;
  if (!granted) { status.textContent = 'Enable access to this site to connect its text fields.'; return; }
  const policy = await broker.resolvePolicy(crypto.randomUUID(), origin);
  command.hidden = policy.suggestionsAllowed;
  status.textContent = policy.paused ? 'Badi is paused. Run badi resume.' : policy.suggestionsAllowed
    ? 'Connected. Type in a text field; Tab requests or accepts, Escape dismisses.'
    : 'Allow this site in your local Badi settings with the command below.';
}

async function connectPage(): Promise<void> {
  await sync();
  if (origin && await permitted(origin)) await activate();
}

enable.addEventListener('click', () => {
  if (!origin || busy) return;
  working(true);
  void chrome.permissions.request({ origins: [hostPattern(origin)] }).then(async granted => {
    if (!granted) { status.textContent = 'Site access was not granted.'; return; }
    await activate();
    await sync();
  }).catch(error => failure(`Connection failed: ${String(error)}`)).finally(() => working(false));
});
disable.addEventListener('click', () => {
  if (!origin || busy) return;
  working(true);
  void chrome.permissions.remove({ origins: [hostPattern(origin)] }).then(sync)
    .catch(() => failure('Could not remove site access.')).finally(() => working(false));
});
retry.addEventListener('click', () => {
  if (!origin || busy) return;
  working(true);
  broker.dispose();
  void connectPage()
    .catch(() => failure('Local model unavailable. Run badi doctor, then retry.'))
    .finally(() => working(false));
});
window.addEventListener('pagehide', () => broker.dispose(), { once: true });
working(true);
void chrome.tabs.query({ active: true, currentWindow: true }).then(async tabs => {
  origin = tabs[0]?.incognito ? null : webOrigin(tabs[0]?.url ?? ''); tabId = tabs[0]?.id;
  site.textContent = origin ?? 'Open a normal website';
  disable.hidden = true;
  if (origin) await connectPage();
  else status.textContent = 'Browser settings, private windows and internal pages are unavailable.';
}).catch(() => failure('Local model unavailable. Run badi doctor, then retry.'))
  .finally(() => working(false));
