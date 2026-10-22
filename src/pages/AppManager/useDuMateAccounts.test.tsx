import { useLayoutEffect } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import * as api from '../../api/tauri';
import type { TKey } from '../../i18n';
import { useNavigationStore } from '../../stores/navigationStore';
import { useDuMateAccounts } from './useDuMateAccounts';
import { dumateCreditBalances, dumatePlanLabel, dumateRewardStatus } from './dumateAccountDisplay';

vi.mock('../../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock('../../components/ConfirmDialog', () => ({ useConfirm: () => async () => true }));
vi.mock('../../api/tauri', () => ({
  listDuMateAccounts: vi.fn(),
  startDuMateLogin: vi.fn(),
  pollDuMateLogin: vi.fn(),
  cancelDuMateLogin: vi.fn().mockResolvedValue(undefined),
  deleteDuMateAccount: vi.fn().mockResolvedValue(undefined),
  refreshDuMateAccountQuota: vi.fn(),
  claimDuMateDailyCredits: vi.fn(),
}));

export const fixture = (): api.DuMateAccount => ({
  id: 'fixture',
  name: 'Fixture',
  enterprise: false,
  plan: 'free',
  active: false,
  expiresAt: null,
  remaining: 5500,
  total: 5500,
  creditBuckets: [
    {
      name: 'grant_point',
      source: 'login_bonus',
      reward: true,
      remaining: 500,
      total: 500,
      expiresAt: null,
    },
    {
      name: 'grant_point',
      source: 'event_bonus',
      reward: true,
      remaining: 5000,
      total: 5000,
      expiresAt: null,
    },
  ],
  dailyReward: null,
});
let state: ReturnType<typeof useDuMateAccounts>;
let renderer: ReactTestRenderer | undefined;
const clearModel = vi.fn();
const showError = vi.fn();
function Harness({ enabled = true, tool = 'dumate' }: { enabled?: boolean; tool?: string }) {
  const result = useDuMateAccounts(enabled, clearModel, showError, tool);
  useLayoutEffect(() => {
    state = result;
  });
  return null;
}
async function mount(enabled = true) {
  act(() => {
    renderer = create(<Harness enabled={enabled} />);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
}
it('collects batch claim errors without a second dialog and keeps individual errors visible', async () => {
  await mount();
  const collect = vi.fn();
  vi.mocked(api.claimDuMateDailyCredits).mockRejectedValue('accountError.claim');
  await act(async () => {
    await state.claimDaily(state.accounts[0], collect);
  });
  expect(collect).toHaveBeenCalledExactlyOnceWith('accountError.claim');
  expect(showError).not.toHaveBeenCalled();
  await act(async () => {
    await state.claimDaily(state.accounts[0]);
  });
  expect(showError).toHaveBeenCalledExactlyOnceWith('accountError.claim');
});
beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers();
  useNavigationStore.getState().setActivePage('apps');
  vi.mocked(api.listDuMateAccounts).mockResolvedValue([fixture()]);
  vi.mocked(api.startDuMateLogin).mockImplementation(async () => ({
    loginId: 'login',
    expiresAt: Date.now() / 1000 + 60,
  }));
  vi.mocked(api.pollDuMateLogin).mockResolvedValue(null);
  vi.mocked(api.refreshDuMateAccountQuota).mockResolvedValue(fixture());
  vi.mocked(api.claimDuMateDailyCredits).mockResolvedValue({
    ...fixture(),
    dailyReward: {
      status: 'CLAIMED_TODAY',
      checkedAt: Date.now() / 1000,
      nextClaimAt: Date.now() / 1000 + 3600,
    },
  });
});
afterEach(() => {
  act(() => renderer?.unmount());
  renderer = undefined;
  vi.useRealTimers();
});

it('navigation only lists installed saved accounts, with a positive refresh control', async () => {
  await mount(false);
  expect(api.listDuMateAccounts).not.toHaveBeenCalled();
  await act(async () => {
    renderer!.update(<Harness />);
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(api.listDuMateAccounts).toHaveBeenCalledTimes(1);
  await act(async () => {
    useNavigationStore.getState().setActivePage('accounts');
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(api.listDuMateAccounts).toHaveBeenCalledTimes(2);
  expect(api.startDuMateLogin).not.toHaveBeenCalled();
  expect(api.refreshDuMateAccountQuota).not.toHaveBeenCalled();
  expect(api.claimDuMateDailyCredits).not.toHaveBeenCalled();
  await act(async () => {
    await state.refresh(state.accounts[0]);
  });
  expect(api.refreshDuMateAccountQuota).toHaveBeenCalledExactlyOnceWith('fixture');
});
it('keeps cached rows and stays silent after a passive read failure', async () => {
  await mount();
  vi.mocked(api.listDuMateAccounts).mockRejectedValueOnce('accountError.read');
  await act(async () => {
    useNavigationStore.getState().setActivePage('accounts');
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(state.accounts).toEqual([fixture()]);
  expect(showError).not.toHaveBeenCalled();
});
it('selection clears the API model, and reward and refresh actions preserve selection', async () => {
  await mount();
  act(() => state.select('fixture'));
  expect(clearModel).toHaveBeenCalled();
  await act(async () => {
    await state.claimDaily(state.accounts[0]);
  });
  expect(state.selectedId).toBe('fixture');
  expect(api.claimDuMateDailyCredits).toHaveBeenCalledTimes(1);
  await act(async () => {
    await state.claimDaily(state.accounts[0]);
  });
  expect(api.claimDuMateDailyCredits).toHaveBeenCalledTimes(1);
  await act(async () => {
    await state.refresh(state.accounts[0]);
  });
  expect(state.selectedId).toBe('fixture');
});
it('leaving cancels login and ignores a late result without native apply', async () => {
  let finish!: (account: api.DuMateAccount) => void;
  vi.mocked(api.pollDuMateLogin).mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  await mount();
  let adding!: Promise<void>;
  await act(async () => {
    adding = state.add();
    await Promise.resolve();
  });
  expect(api.startDuMateLogin).toHaveBeenCalledTimes(1);
  act(() => renderer!.update(<Harness enabled={false} tool="other" />));
  await act(async () => {
    finish({ ...fixture(), id: 'late' });
    await adding;
  });
  expect(api.cancelDuMateLogin).toHaveBeenCalledWith('login');
  expect(state.accounts.some((a) => a.id === 'late')).toBe(false);
});
it('expires login after sixty seconds', async () => {
  await mount();
  let adding!: Promise<void>;
  await act(async () => {
    adding = state.add();
    await vi.advanceTimersByTimeAsync(60_100);
    await adding;
  });
  expect(api.cancelDuMateLogin).toHaveBeenCalledWith('login');
  expect(showError).toHaveBeenCalledWith('accountError.expired');
  expect(state.busy).toBe(false);
});
it('deleting an account uses the shared confirmation flow and does not query quota', async () => {
  await mount();
  act(() => state.select('fixture'));
  await act(async () => {
    await state.remove(state.accounts[0]);
  });
  expect(api.deleteDuMateAccount).toHaveBeenCalledExactlyOnceWith('fixture');
  expect(state.accounts).toHaveLength(0);
  expect(api.refreshDuMateAccountQuota).not.toHaveBeenCalled();
});
it('preserves real bucket capacities, unknown values, daily rollover and enterprise identity', () => {
  const a = fixture();
  const t = (key: TKey) => key;
  expect(dumateCreditBalances(a)).toMatchObject({
    package: { remaining: 0, total: 0 },
    reward: { remaining: 5500, total: 5500 },
  });
  a.remaining = null;
  a.total = null;
  a.creditBuckets = [];
  expect(dumateCreditBalances(a).package.remaining).toBeNull();
  a.dailyReward = { status: 'CLAIMED_TODAY', checkedAt: 100, nextClaimAt: 200 };
  expect(dumateRewardStatus(a, 150)).toBe('claimed');
  expect(dumateRewardStatus(a, 200)).toBe('unknown');
  for (const [plan, label] of [
    ['free', 'agent.qwenworkFree'],
    ['pro', 'Pro'],
    ['max', 'Max'],
    ['team', 'agent.dumateEnterpriseSeat'],
  ] as const) {
    expect(dumatePlanLabel({ ...a, plan }, t)).toBe(label);
  }
  expect(dumateRewardStatus({ ...a, enterprise: true }, 150)).toBe('unavailable');
});
