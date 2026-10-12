import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { AccountProxyPanel } from './AccountProxyPanel';
import * as api from '../../api/accountProxy';
import type { AccountProvider } from '../../api/types';
import { useNavigationStore } from '../../stores/navigationStore';

const { context, confirm } = vi.hoisted(() => ({
  confirm: vi.fn(),
  context: {
    setNewModelForm: vi.fn(),
    setEditingModelId: vi.fn(),
    setKeyDestroyed: vi.fn(),
    setShowApiKey: vi.fn(),
    setModelModalDestination: vi.fn(),
    setShowAddModelModal: vi.fn(),
  },
}));
vi.mock('../../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock('../../components/ConfirmDialog', () => ({ useConfirm: () => confirm }));
vi.mock('../ModelNexus/context', () => ({ useModelNexus: () => context }));
vi.mock('../../api/accountProxy', async (original) => ({
  ...(await original<typeof import('../../api/accountProxy')>()),
  listProxyAccounts: vi.fn(),
  getAccountProxyModels: vi.fn(),
}));

let renderer: ReactTestRenderer;
function accounts(provider: AccountProvider, count: number): api.ProxyAccount[] {
  return Array.from({ length: count }, (_, index) => ({
    source: { provider, accountId: `saved-${index}` },
    label: `Saved ${index}`,
  }));
}
async function open() {
  await act(async () => {
    renderer = create(<AccountProxyPanel acknowledged onAcknowledgedChange={vi.fn()} />);
  });
}
async function navigate(page: 'accounts' | 'freeModels') {
  await act(async () => useNavigationStore.getState().setActivePage(page));
}
function row(provider: AccountProvider) {
  const name = api.accountProxyProviders.find((entry) => entry.id === provider)!.name;
  return renderer.root
    .findAllByType('button')
    .find((button) => button.props['aria-label']?.endsWith(`: ${name}`))!;
}
function count(provider: AccountProvider) {
  return row(provider).findAllByType('span')[1].children.join('');
}
beforeEach(() => {
  useNavigationStore.setState({ activePage: 'freeModels' });
  vi.mocked(api.listProxyAccounts)
    .mockReset()
    .mockImplementation(async (provider) =>
      accounts(provider, provider === 'claudeCode' ? 0 : provider === 'codex' ? 2 : 1)
    );
});
afterEach(() => {
  act(() => renderer?.unmount());
  vi.clearAllMocks();
});

it('counts saved accounts independently, including zero, without reading model catalogs or changing selections', async () => {
  await open();
  expect(vi.mocked(api.listProxyAccounts).mock.calls.map(([provider]) => provider)).toEqual(
    api.accountProxyProviders.map((provider) => provider.id)
  );
  expect(count('claudeCode')).toBe('0 个账号');
  expect(count('codex')).toBe('2 个账号');
  expect(count('kilo')).toBe('1 个账号');
  expect(count('workBuddy')).toBe('1 个账号');
  expect(api.getAccountProxyModels).not.toHaveBeenCalled();
  expect(context.setNewModelForm).not.toHaveBeenCalled();
  expect(context.setShowAddModelModal).not.toHaveBeenCalled();
  act(() => row('kilo').props.onClick());
  expect(context.setNewModelForm).toHaveBeenCalledExactlyOnceWith(
    expect.objectContaining({ accountSource: { provider: 'kilo', accountId: '' } })
  );
  expect(context.setModelModalDestination).toHaveBeenCalledWith('freeRouter');
  expect(context.setShowAddModelModal).toHaveBeenCalledWith(true);
  expect(api.listProxyAccounts).toHaveBeenCalledTimes(7);
});

it('reloads counts when returning from account management, preserves known counts on failure and updates after deletion', async () => {
  await open();
  await navigate('accounts');
  vi.mocked(api.listProxyAccounts)
    .mockClear()
    .mockImplementation(async (provider) => {
      if (provider === 'codex') throw new Error('Saved store temporarily unavailable');
      return accounts(provider, 0);
    });
  expect(api.listProxyAccounts).not.toHaveBeenCalled();
  await navigate('freeModels');
  expect(count('codex')).toBe('2 个账号');
  expect(count('kilo')).toBe('0 个账号');
  await navigate('accounts');
  vi.mocked(api.listProxyAccounts).mockImplementation(async (provider) =>
    accounts(provider, provider === 'codex' ? 3 : 0)
  );
  await navigate('freeModels');
  expect(count('codex')).toBe('3 个账号');
  expect(api.getAccountProxyModels).not.toHaveBeenCalled();
  expect(context.setNewModelForm).not.toHaveBeenCalled();
});

it('shows unknown rather than a false zero for an unreadable saved account list', async () => {
  vi.mocked(api.listProxyAccounts).mockImplementation(async (provider) => {
    if (provider === 'codex') throw new Error('Unreadable store');
    return [];
  });
  await open();
  expect(count('codex')).toBe('—');
  expect(count('kilo')).toBe('0 个账号');
  expect(api.getAccountProxyModels).not.toHaveBeenCalled();
});

it('ignores an earlier directory after leaving the page and does not load while hidden', async () => {
  useNavigationStore.setState({ activePage: 'accounts' });
  await open();
  expect(api.listProxyAccounts).not.toHaveBeenCalled();
  let resolve!: (rows: api.ProxyAccount[]) => void;
  vi.mocked(api.listProxyAccounts).mockImplementation((provider) =>
    provider === 'codex'
      ? new Promise((done) => {
          resolve = done;
        })
      : Promise.resolve([])
  );
  await navigate('freeModels');
  expect(count('codex')).toBe('—');
  await navigate('accounts');
  vi.mocked(api.listProxyAccounts).mockImplementation(async (provider) =>
    accounts(provider, provider === 'codex' ? 1 : 0)
  );
  await navigate('freeModels');
  expect(count('codex')).toBe('1 个账号');
  await act(async () => resolve(accounts('codex', 9)));
  expect(count('codex')).toBe('1 个账号');
  expect(api.getAccountProxyModels).not.toHaveBeenCalled();
  expect(context.setShowAddModelModal).not.toHaveBeenCalled();
});
