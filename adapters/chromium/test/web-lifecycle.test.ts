import { describe, expect, it, vi } from 'vitest';
import { retireUnpermittedRoutes } from '../src/background/browser-worker';
import type { TrustedSessionRouteEntry } from '../src/background/session-routes';

function entry(sessionId: string, origin: string, tabId: number): TrustedSessionRouteEntry {
  return { sessionId, route: { origin, tabId, frameId: 0, documentId: `document-${sessionId}` } };
}

describe('ordinary website permission revocation', () => {
  it('revokes every affected document while other enabled sites keep working', async () => {
    const first = entry('one', 'https://revoked.example', 1);
    const second = entry('two', 'https://kept.example', 2);
    const third = entry('three', 'https://revoked.example', 3);
    const retire = vi.fn().mockResolvedValue(undefined);
    await retireUnpermittedRoutes([first, second, third],
      async origin => origin === 'https://kept.example', retire);
    expect(retire.mock.calls.map(([item]) => item.sessionId).sort()).toEqual(['one', 'three']);
  });

  it('fails closed on permission lookup failure without skipping other documents', async () => {
    const retire = vi.fn().mockResolvedValue(undefined);
    await retireUnpermittedRoutes([entry('one', 'https://first.example', 1),
      entry('two', 'https://second.example', 2)], async () => { throw new Error('permission API unavailable'); }, retire);
    expect(retire).toHaveBeenCalledTimes(2);
  });
});
