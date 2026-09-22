export function webOrigin(value: string): string | null {
  try {
    const url = new URL(value);
    return ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password ? url.origin : null;
  } catch { return null; }
}

export function hostPattern(origin: string): string {
  const url = new URL(origin);
  if (webOrigin(origin) !== origin) throw new Error('Expected a canonical web origin');
  return `${url.protocol}//${url.hostname}/*`;
}

export function trustedBootstrap(sender: chrome.runtime.MessageSender, id: string): boolean {
  return sender.id === id && sender.frameId === 0 && typeof sender.documentId === 'string' &&
    sender.documentId.length > 0 && sender.documentLifecycle === 'active' &&
    typeof sender.tab?.id === 'number' && typeof sender.tab.windowId === 'number' &&
    typeof sender.tab.active === 'boolean' && sender.tab.incognito === false &&
    sender.tab.discarded === false && sender.tab.frozen === false &&
    typeof sender.url === 'string' && typeof sender.origin === 'string' &&
    webOrigin(sender.url) === sender.origin;
}

export function trustedSender(sender: chrome.runtime.MessageSender, id: string): boolean {
  return trustedBootstrap(sender, id) && sender.tab?.active === true;
}

export function focusedSender(sender: chrome.runtime.MessageSender, tab: chrome.tabs.Tab,
  window: chrome.windows.Window, id: string): boolean {
  return trustedSender(sender, id) && tab.id === sender.tab?.id && tab.windowId === sender.tab?.windowId &&
    window.id === tab.windowId && window.focused === true && tab.active === true &&
    tab.incognito === false && tab.discarded === false && tab.frozen === false &&
    typeof tab.url === 'string' && webOrigin(tab.url) === sender.origin;
}

export function hostGrantMatches(pattern: string, origin: string): boolean {
  if (webOrigin(origin) !== origin) return false;
  if (pattern === '<all_urls>') return true;
  const match = /^(\*|https?):\/\/([^/]+)\/.*$/u.exec(pattern);
  if (!match) return false;
  const url = new URL(origin);
  const [, scheme, host] = match;
  if (scheme !== '*' && `${scheme}:` !== url.protocol) return false;
  return host === '*' || host === url.hostname || !!host?.startsWith('*.') &&
    (url.hostname === host.slice(2) || url.hostname.endsWith(host.slice(1)));
}

export async function permitted(origin: string): Promise<boolean> {
  if (webOrigin(origin) !== origin) return false;
  // activeTab is temporary; it must not masquerade as persistent site enablement.
  const grants = await chrome.permissions.getAll();
  return !!grants.origins?.some(pattern => hostGrantMatches(pattern, origin)) &&
    chrome.permissions.contains({ origins: [hostPattern(origin)] });
}
