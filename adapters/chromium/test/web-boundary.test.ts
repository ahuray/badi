import { describe, expect, it, vi } from 'vitest';
import { focusedSender, hostGrantMatches, hostPattern, permitted, trustedBootstrap, webOrigin } from '../src/web/boundary';
import { normalizeWireVersion } from '../src/background/native-client';

const sender = { id: 'badi', frameId: 0, documentId: 'document', documentLifecycle: 'active',
  origin: 'https://example.com', url: 'https://example.com/note',
  tab: { id: 1, windowId: 2, active: true, incognito: false, discarded: false, frozen: false },
} as chrome.runtime.MessageSender;

describe('ordinary website boundary', () => {
  it('requires a persistent matching host grant beyond temporary activeTab access', async () => {
    const getAll = vi.fn().mockResolvedValue({ origins: ['https://other.example/*'] });
    const contains = vi.fn().mockResolvedValue(true);
    vi.stubGlobal('chrome', { permissions: { getAll, contains } });
    try {
      expect(await permitted('https://example.com')).toBe(false);
      expect(contains).not.toHaveBeenCalled();
      getAll.mockResolvedValue({ origins: ['https://example.com/*'] });
      expect(await permitted('https://example.com:8443')).toBe(true);
      contains.mockResolvedValue(false);
      expect(await permitted('https://example.com')).toBe(false);
    } finally { vi.unstubAllGlobals(); }
  });
  it('matches Chrome host grants without crossing schemes or domain suffix boundaries', () => {
    expect(hostGrantMatches('https://*.example.com/*', 'https://example.com')).toBe(true);
    expect(hostGrantMatches('https://*.example.com/*', 'https://notes.example.com:8443')).toBe(true);
    expect(hostGrantMatches('https://*.example.com/*', 'https://notexample.com')).toBe(false);
    expect(hostGrantMatches('https://example.com/*', 'http://example.com')).toBe(false);
    expect(hostGrantMatches('*://*/*', 'https://example.com')).toBe(true);
    expect(hostGrantMatches('<all_urls>', 'file:///tmp/note')).toBe(false);
    expect(hostGrantMatches('http://[::1]/*', 'http://[::1]:9000')).toBe(true);
  });
  it('binds a live normal top-level document to its exact origin', () => {
    expect(trustedBootstrap(sender, 'badi')).toBe(true);
    for (const change of [{ frameId: 1 }, { origin: 'https://other.example' }, { documentLifecycle: 'cached' },
      { documentId: '' }, { id: 'other' }, { url: 'about:blank' },
      { tab: { ...sender.tab, incognito: true } }]) {
      expect(trustedBootstrap({ ...sender, ...change } as chrome.runtime.MessageSender, 'badi')).toBe(false);
    }
  });
  it('rechecks current tab and actual browser window focus', () => {
    const tab = { ...sender.tab, url: sender.url } as chrome.tabs.Tab;
    expect(focusedSender(sender, tab, { id: 2, focused: true } as chrome.windows.Window, 'badi')).toBe(true);
    expect(focusedSender(sender, tab, { id: 2, focused: false } as chrome.windows.Window, 'badi')).toBe(false);
    expect(focusedSender(sender, { ...tab, url: 'https://other.example' }, { id: 2, focused: true } as chrome.windows.Window, 'badi')).toBe(false);
  });
  it('keeps credentials and internal URLs out of origins and host grants', () => {
    for (const value of ['chrome://settings', 'file:///tmp/note', 'https://user:secret@example.com']) expect(webOrigin(value)).toBeNull();
    expect(webOrigin('https://example.com:8443/path?q=1')).toBe('https://example.com:8443');
    expect(hostPattern('https://example.com:8443')).toBe('https://example.com/*');
    expect(() => hostPattern('https://example.com/path')).toThrow();
  });
  it('requires the exact negotiated protocol on every frame and hello', () => {
    expect(normalizeWireVersion({ v: 1, type: 'suggestion.show' }, 2)).toBeNull();
    expect(normalizeWireVersion({ v: 2, type: 'hello.ack', payload: { selected_v: 1 } }, 2)).toBeNull();
    expect(normalizeWireVersion({ v: 2, type: 'hello.ack', payload: { selected_v: 2, paused: false } }, 2))
      .toEqual({ v: 1, type: 'hello.ack', payload: { selected_v: 1, paused: false } });
  });
});
