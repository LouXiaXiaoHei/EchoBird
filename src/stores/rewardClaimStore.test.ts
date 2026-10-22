import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const key = 'echobird_auto_claim_rewards';
const saved = new Map<string, string>();
const storage = {
  getItem: vi.fn((name: string) => saved.get(name) ?? null),
  setItem: vi.fn((name: string, value: string) => saved.set(name, value)),
};

beforeEach(() => {
  vi.resetModules();
  saved.clear();
  vi.clearAllMocks();
  vi.stubGlobal('localStorage', storage);
});
afterEach(() => vi.unstubAllGlobals());

it.each([null, 'false', 'unexpected'])(
  'defaults off for %s without writing settings',
  async (value) => {
    if (value !== null) saved.set(key, value);
    const { useRewardClaimStore } = await import('./rewardClaimStore');
    expect(useRewardClaimStore.getState().enabled).toBe(false);
    expect(storage.setItem).not.toHaveBeenCalled();
  }
);

it('persists explicit opt-in and opt-out across restarts', async () => {
  const { useRewardClaimStore } = await import('./rewardClaimStore');
  useRewardClaimStore.getState().setEnabled(true);
  expect(storage.setItem).toHaveBeenLastCalledWith(key, 'true');
  vi.resetModules();
  const restarted = (await import('./rewardClaimStore')).useRewardClaimStore;
  expect(restarted.getState().enabled).toBe(true);
  restarted.getState().setEnabled(false);
  vi.resetModules();
  expect((await import('./rewardClaimStore')).useRewardClaimStore.getState().enabled).toBe(false);
});

it('keeps disabled as the safe default when storage is unavailable', async () => {
  storage.getItem.mockImplementationOnce(() => {
    throw new Error('unavailable');
  });
  storage.setItem.mockImplementationOnce(() => {
    throw new Error('unavailable');
  });
  const { useRewardClaimStore } = await import('./rewardClaimStore');
  expect(useRewardClaimStore.getState().enabled).toBe(false);
  expect(() => useRewardClaimStore.getState().setEnabled(true)).not.toThrow();
  expect(useRewardClaimStore.getState().enabled).toBe(true);
});
