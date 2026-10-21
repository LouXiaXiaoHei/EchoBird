import { useLayoutEffect } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import * as api from '../../api/devin';
import { openExternal } from '../../api/tauri';
import { useDevinAccounts } from './useDevinAccounts';

vi.mock('../../api/devin', () => ({
  listDevinAccounts: vi.fn(),
  startDevinLogin: vi.fn(),
  pollDevinLogin: vi.fn(),
  cancelDevinLogin: vi.fn().mockResolvedValue(undefined),
  deleteDevinAccount: vi.fn().mockResolvedValue(undefined),
  refreshDevinAccount: vi.fn(),
  switchDevinAccount: vi.fn(),
}));
vi.mock('../../api/tauri', () => ({ openExternal: vi.fn().mockResolvedValue(undefined) }));
vi.mock('../../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock('../../components/ConfirmDialog', () => ({ useConfirm: () => async () => true }));
const account: api.DevinAccount = {
  id: 'one',
  email: 'one@example.test',
  active: true,
  plan: 'Free',
  quotaWindows: [],
};
let state: ReturnType<typeof useDevinAccounts>;
let renderer: ReactTestRenderer;
const error = vi.fn();
function Harness({ tool = 'devin', enabled = true }: { tool?: api.DevinTool; enabled?: boolean }) {
  const group = useDevinAccounts(tool, enabled, error, tool);
  useLayoutEffect(() => {
    state = group;
  });
  return null;
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
async function mount(enabled = true) {
  act(() => {
    renderer = create(<Harness enabled={enabled} />);
  });
  await act(async () => {
    await vi.runOnlyPendingTimersAsync();
  });
}
beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers();
  vi.mocked(api.listDevinAccounts).mockResolvedValue([account]);
  vi.mocked(api.startDevinLogin).mockResolvedValue({
    loginId: 'login',
    verificationUri: 'https://app.devin.ai/auth/cli/continue',
    expiresAt: Date.now() / 1000 + 60,
  });
  vi.mocked(api.pollDevinLogin).mockResolvedValue(null);
  vi.mocked(api.refreshDevinAccount).mockResolvedValue(account);
});
afterEach(() => {
  act(() => renderer?.unmount());
  vi.useRealTimers();
});
it('loads installed native snapshots only, and independently visits the two applications', async () => {
  await mount(false);
  expect(api.listDevinAccounts).not.toHaveBeenCalled();
  act(() => renderer.update(<Harness />));
  await act(async () => {
    await vi.runOnlyPendingTimersAsync();
  });
  expect(api.listDevinAccounts).toHaveBeenLastCalledWith('devin');
  expect(state.selectedId).toBe('one');
  act(() => renderer.update(<Harness tool="devincli" />));
  await act(async () => {
    await vi.runOnlyPendingTimersAsync();
  });
  expect(api.listDevinAccounts).toHaveBeenLastCalledWith('devincli');
  expect(api.refreshDevinAccount).not.toHaveBeenCalled();
  expect(api.startDevinLogin).not.toHaveBeenCalled();
  expect(api.switchDevinAccount).not.toHaveBeenCalled();
});
it('keeps cached rows silently when native configuration becomes unreadable', async () => {
  await mount();
  vi.mocked(api.listDevinAccounts).mockRejectedValueOnce(new Error('accountError.read'));
  act(() => renderer.update(<Harness tool="devincli" />));
  await act(async () => {
    await vi.runOnlyPendingTimersAsync();
  });
  expect(state.accounts).toEqual([account]);
  expect(error).not.toHaveBeenCalled();
});
it('cancels a pending browser login on application change and ignores its late account', async () => {
  await mount();
  const poll = deferred<api.DevinAccount | null>();
  vi.mocked(api.pollDevinLogin).mockReturnValue(poll.promise);
  let adding!: Promise<void>;
  await act(async () => {
    adding = state.add();
  });
  expect(openExternal).toHaveBeenCalledOnce();
  expect(state.busy).toBe(true);
  act(() => renderer.update(<Harness tool="devincli" />));
  await act(async () => {
    poll.resolve({ ...account, id: 'late' });
    await adding;
    await vi.runOnlyPendingTimersAsync();
  });
  expect(api.cancelDevinLogin).toHaveBeenCalledWith('login');
  expect(state.accounts).toEqual([account]);
  expect(api.switchDevinAccount).not.toHaveBeenCalled();
});
it('expires the browser login after sixty seconds without accepting a late response', async () => {
  await mount();
  const poll = deferred<api.DevinAccount | null>();
  vi.mocked(api.pollDevinLogin).mockReturnValue(poll.promise);
  let adding!: Promise<void>;
  await act(async () => {
    adding = state.add();
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000);
  });
  expect(state.busy).toBe(false);
  expect(error).toHaveBeenCalledWith('accountError.expired');
  expect(api.cancelDevinLogin).toHaveBeenCalledWith('login');
  await act(async () => {
    poll.resolve({ ...account, id: 'late' });
    await adding;
  });
  expect(state.accounts).toEqual([account]);
});
it('refreshes only on request and removes only from the saved pool', async () => {
  await mount();
  await act(async () => {
    await state.refresh(account);
  });
  expect(api.refreshDevinAccount).toHaveBeenCalledExactlyOnceWith('one');
  expect(state.selectedId).toBe('one');
  await act(async () => {
    await state.remove(account);
  });
  expect(api.deleteDevinAccount).toHaveBeenCalledExactlyOnceWith('one');
  expect(api.switchDevinAccount).not.toHaveBeenCalled();
});
