import { useLayoutEffect } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import * as api from '../../api/tauri';
import { useNavigationStore } from '../../stores/navigationStore';
import { useQwenWorkAccounts } from './useQwenWorkAccounts';
import { qwenWorkPlanLabel } from './qwenWorkAccountDisplay';

vi.mock('../../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock('../../components/ConfirmDialog', () => ({ useConfirm: () => async () => true }));
vi.mock('../../api/tauri', () => ({
  listQwenWorkAccounts: vi.fn(),
  startQwenWorkLogin: vi.fn(),
  pollQwenWorkLogin: vi.fn(),
  cancelQwenWorkLogin: vi.fn().mockResolvedValue(undefined),
  openExternal: vi.fn().mockResolvedValue(undefined),
  deleteQwenWorkAccount: vi.fn().mockResolvedValue(undefined),
  refreshQwenWorkAccountQuota: vi.fn(),
}));

let state: ReturnType<typeof useQwenWorkAccounts>;
let renderer: ReactTestRenderer;
const clearModel = vi.fn();
const showError = vi.fn();
const account = (edition: api.QwenWorkEdition): api.QwenWorkAccount => ({
  id: edition,
  edition,
  name: 'Fixture',
  email: null,
  organization: 'Fixture organization',
  plan: 'qwen-office-pro',
  planName: 'Pro',
  expiresAt: null,
  active: true,
  remaining: null,
  total: null,
  creditBuckets: [],
  dailyReward: null,
});
function Harness({
  edition = 'qwenworkcn',
  enabled = true,
}: {
  edition?: api.QwenWorkEdition;
  enabled?: boolean;
}) {
  const result = useQwenWorkAccounts(edition, enabled, clearModel, showError, edition);
  useLayoutEffect(() => {
    state = result;
  });
  return null;
}
async function mount(edition: api.QwenWorkEdition = 'qwenworkcn', enabled = true) {
  act(() => {
    renderer = create(<Harness edition={edition} enabled={enabled} />);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
}
beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers();
  useNavigationStore.getState().setActivePage('apps');
  vi.mocked(api.listQwenWorkAccounts).mockImplementation(async (edition) => [account(edition)]);
  vi.mocked(api.startQwenWorkLogin).mockImplementation(async (edition) => ({
    loginId: edition,
    verificationUri: `https://example.test/${edition}`,
    expiresAt: Date.now() / 1000 + 60,
  }));
  vi.mocked(api.pollQwenWorkLogin).mockResolvedValue(null);
  vi.mocked(api.refreshQwenWorkAccountQuota).mockImplementation(async (edition) => ({
    ...account(edition),
    remaining: 42,
  }));
});
afterEach(() => {
  act(() => {
    renderer?.unmount();
  });
  vi.useRealTimers();
});

it.each(['qwenworkcn', 'qwenwork'] as const)(
  '%s navigation only lists saved rows; explicit refresh keeps selection',
  async (edition) => {
    await mount(edition);
    expect(state.selectedId).toBe(edition);
    expect(state.accounts[0].remaining).toBeNull();
    expect(api.refreshQwenWorkAccountQuota).not.toHaveBeenCalled();
    expect(api.startQwenWorkLogin).not.toHaveBeenCalled();
    await act(async () => {
      await state.refresh(state.accounts[0]);
    });
    expect(api.refreshQwenWorkAccountQuota).toHaveBeenCalledExactlyOnceWith(edition, edition);
    expect(state.accounts[0].remaining).toBe(42);
    expect(state.selectedId).toBe(edition);
    expect(showError).not.toHaveBeenCalled();
  }
);
it('uninstalled clients do not load or start authentication', async () => {
  await mount('qwenworkcn', false);
  await act(async () => {
    await state.add();
  });
  expect(api.listQwenWorkAccounts).not.toHaveBeenCalled();
  expect(api.startQwenWorkLogin).not.toHaveBeenCalled();
});
it('switching editions cancels a pending login and ignores the late result', async () => {
  await mount();
  let resolve!: (value: api.QwenWorkAccount) => void;
  vi.mocked(api.pollQwenWorkLogin).mockImplementation(
    () =>
      new Promise((done) => {
        resolve = done;
      })
  );
  let pending!: Promise<void>;
  act(() => {
    pending = state.add();
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(api.openExternal).toHaveBeenCalledOnce();
  act(() => {
    renderer.update(<Harness edition="qwenwork" />);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(api.cancelQwenWorkLogin).toHaveBeenCalledWith('qwenworkcn');
  await act(async () => {
    resolve(account('qwenworkcn'));
    await pending;
  });
  expect(state.accounts.map((a) => a.edition)).toEqual(['qwenwork']);
  expect(state.busy).toBe(false);
  expect(api.refreshQwenWorkAccountQuota).not.toHaveBeenCalled();
});
it('passive loading failures retain cached rows without dialogs or quota requests', async () => {
  await mount();
  vi.mocked(api.listQwenWorkAccounts).mockRejectedValueOnce('accountError.read');
  act(() => {
    useNavigationStore.getState().setActivePage('accounts');
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(state.accounts[0].id).toBe('qwenworkcn');
  expect(state.readFailed).toBe(true);
  expect(showError).not.toHaveBeenCalled();
  expect(api.refreshQwenWorkAccountQuota).not.toHaveBeenCalled();
});
it('maps the five public plans and preserves unknown plans', () => {
  const t = (key: string) => key;
  for (const [plan, key] of [
    ['free', 'Free'],
    ['qwen-office-plus', 'Plus'],
    ['qwen-office-pro', 'Pro'],
    ['enterprise_standard', 'EnterpriseStandard'],
    ['enterprise_premium', 'EnterprisePremium'],
  ]) {
    expect(qwenWorkPlanLabel({ edition: 'qwenworkcn', plan, planName: null }, t)).toBe(
      `agent.qwenwork${key}`
    );
    expect(qwenWorkPlanLabel({ edition: 'qwenwork', plan, planName: null }, t)).toBe(
      key === 'EnterpriseStandard'
        ? 'Enterprise Standard'
        : key === 'EnterprisePremium'
          ? 'Enterprise Premium'
          : key
    );
  }
  expect(
    qwenWorkPlanLabel({ edition: 'qwenworkcn', plan: 'future-tier', planName: 'Future tier' }, t)
  ).toBe('Future tier');
  expect(
    qwenWorkPlanLabel(
      { edition: 'qwenworkcn', plan: 'qwen-office-enterprise', planName: '企业旗舰版' },
      t
    )
  ).toBe('agent.qwenworkEnterprisePremium');
});
