import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const key = 'echobird-router-account-names-hidden';
let saved: Map<string, string>;
let storage: { getItem: ReturnType<typeof vi.fn>; setItem: ReturnType<typeof vi.fn> };

beforeEach(() => {
  vi.resetModules();
  saved = new Map();
  storage = {
    getItem: vi.fn((key: string) => saved.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => saved.set(key, value)),
  };
  vi.stubGlobal('localStorage', storage);
});
afterEach(() => vi.unstubAllGlobals());

describe('Shared account privacy', () => {
  it.each([null, 'false', 'true'])(
    'reads the existing %s preference without a passive write',
    async (value) => {
      if (value !== null) saved.set(key, value);
      const { useAccountPrivacyStore } = await import('./accountPrivacyStore');
      expect(useAccountPrivacyStore.getState().hidden).toBe(value === 'true');
      expect(storage.setItem).not.toHaveBeenCalled();
    }
  );

  it('notifies all three views and restores the shared choice after a fresh module load', async () => {
    const { useAccountPrivacyStore } = await import('./accountPrivacyStore');
    const router = vi.fn();
    const accountCenter = vi.fn();
    const desktop = vi.fn();
    const unsubscribes = [
      useAccountPrivacyStore.subscribe(router),
      useAccountPrivacyStore.subscribe(accountCenter),
      useAccountPrivacyStore.subscribe(desktop),
    ];
    useAccountPrivacyStore.getState().setHidden(true);
    expect(router).toHaveBeenCalledOnce();
    expect(accountCenter).toHaveBeenCalledOnce();
    expect(desktop).toHaveBeenCalledOnce();
    expect(storage.setItem).toHaveBeenCalledExactlyOnceWith(key, 'true');
    unsubscribes.forEach((unsubscribe) => unsubscribe());
    storage.setItem.mockClear();
    vi.resetModules();
    const restarted = (await import('./accountPrivacyStore')).useAccountPrivacyStore;
    expect(restarted.getState().hidden).toBe(true);
    expect(storage.setItem).not.toHaveBeenCalled();
  });

  it('still toggles in memory when reading or writing preferences fails', async () => {
    storage.getItem.mockImplementation(() => {
      throw new Error('unavailable');
    });
    storage.setItem.mockImplementation(() => {
      throw new Error('unavailable');
    });
    const { useAccountPrivacyStore } = await import('./accountPrivacyStore');
    expect(useAccountPrivacyStore.getState().hidden).toBe(false);
    expect(() => useAccountPrivacyStore.getState().setHidden(true)).not.toThrow();
    expect(useAccountPrivacyStore.getState().hidden).toBe(true);
    useAccountPrivacyStore.getState().setHidden(false);
    expect(useAccountPrivacyStore.getState().hidden).toBe(false);
  });
});
