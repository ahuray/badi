import { startBrowserWorker } from '../background/browser-worker';
import { focusedSender, permitted, trustedBootstrap, trustedSender } from './boundary';

startBrowserWorker({ bootstrap: trustedBootstrap, sender: trustedSender, focused: focusedSender, permitted }, 2);

let registration = Promise.resolve();
function synchronize(): Promise<void> {
  registration = registration.catch(() => undefined).then(async () => {
    const grants = await chrome.permissions.getAll();
    const matches = (grants.origins ?? []).filter(origin => /^https?:\/\//u.test(origin));
    const existing = await chrome.scripting.getRegisteredContentScripts({ ids: ['badi-web'] });
    if (!matches.length) {
      if (existing.length) await chrome.scripting.unregisterContentScripts({ ids: ['badi-web'] });
    } else if (existing.length) {
      await chrome.scripting.updateContentScripts([{ id: 'badi-web', matches, js: ['web-content-script.js'] }]);
    } else {
      await chrome.scripting.registerContentScripts([{ id: 'badi-web', matches,
        js: ['web-content-script.js'], runAt: 'document_idle', allFrames: false,
        persistAcrossSessions: true }]);
    }
  });
  return registration;
}
function refresh(): void {
  void synchronize().catch(error => console.error('Badi site registration failed', String(error)));
}
chrome.runtime.onInstalled.addListener(refresh);
chrome.runtime.onStartup.addListener(refresh);
chrome.permissions.onAdded.addListener(refresh);
chrome.permissions.onRemoved.addListener(refresh);
