import { useLayoutEffect } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { DuMateAccount } from '../../api/dumate';
import { useToolsStore } from '../../stores/toolsStore';
import { useNavigationStore } from '../../stores/navigationStore';
import { AppManagerProvider } from './AppManagerProvider';
import { useAppManager } from './context';
import { DuMateAccountSection } from './DuMateAccountSection';
import { AccountSectionRow } from './AccountSectionPrimitives';
import { accountProxyStatus } from './accountProxyStatus';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.hoisted(() => vi.stubGlobal('__APP_EDITION__', 'full'));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../../hooks/useI18n', () => ({
  useI18n: () => ({ t: (key: string) => key, locale: 'en' }),
}));
vi.mock('../../components/ConfirmDialog', () => ({ useConfirm: () => async () => true }));
vi.mock('../ModelNexus/context', () => {
  const userModels: never[] = [];
  return { useModelNexus: () => ({ userModels }) };
});
vi.mock('../FreeModels/provider', () => ({ useFreeModels: () => ({ routerEnabled: false }) }));
vi.mock('./ClaudeCodeLoginDialog', () => ({ ClaudeCodeLoginDialog: () => null }));

const row: DuMateAccount = {
  id: 'saved',
  name: 'Fixture',
  enterprise: false,
  active: true,
  plan: 'max',
  expiresAt: null,
  remaining: 200,
  total: 500,
  creditBuckets: [],
  dailyReward: null,
};
let state: ReturnType<typeof useAppManager>;
let renderer: ReactTestRenderer;
let handlers: Map<string, () => unknown>;
function Harness() {
  const context = useAppManager();
  useLayoutEffect(() => {
    state = context;
  });
  return context.selectedTool === 'dumate' ? <DuMateAccountSection /> : null;
}
async function tick() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1);
  });
}
async function mount(installed = true) {
  useToolsStore
    .getState()
    .setDetectedTools([
      { id: 'dumate', name: 'DuMate', category: 'Desktop', installed, noModelConfig: true },
    ]);
  act(() => {
    renderer = create(
      <AppManagerProvider>
        <Harness />
      </AppManagerProvider>
    );
  });
  act(() => state.setSelectedTool('dumate'));
  await tick();
}
const calls = (name: string) => invoke.mock.calls.filter(([command]) => command === name);
beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  const storage = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  });
  useNavigationStore.getState().setActivePage('apps');
  handlers = new Map<string, () => unknown>([
    ['get_models', () => []],
    ['get_tool_install_index', () => '{"ids":[]}'],
    ['list_dumate_accounts', () => [row]],
    ['switch_dumate_account', () => row],
    ['refresh_dumate_account_quota', () => row],
    ['start_tool', () => ({ success: true })],
  ]);
  invoke.mockImplementation(async (name: string) => {
    const handler = handlers.get(name);
    if (!handler) throw new Error(`Unexpected IPC ${name}`);
    return handler();
  });
});
afterEach(() => {
  act(() => renderer?.unmount());
  useToolsStore.getState().setDetectedTools([]);
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('actual app entry uses saved rows and only explicit apply switches then launches DuMate', async () => {
  await mount();
  expect(renderer.root.findAllByType(AccountSectionRow)).toHaveLength(1);
  expect(calls('list_dumate_accounts')).toHaveLength(1);
  expect(calls('refresh_dumate_account_quota')).toHaveLength(0);
  expect(calls('switch_dumate_account')).toHaveLength(0);
  expect(calls('start_tool')).toHaveLength(0);
  await act(async () => {
    await state.handleLaunch();
  });
  expect(calls('switch_dumate_account')).toEqual([
    ['switch_dumate_account', { accountId: 'saved' }],
  ]);
  expect(calls('start_tool')).toEqual([
    ['start_tool', { toolId: 'dumate', startCommand: null, cwd: null }],
  ]);
  const commands = invoke.mock.calls.map(([command]) => command);
  expect(commands.indexOf('switch_dumate_account')).toBeLessThan(commands.indexOf('start_tool'));
});
it('uninstalled DuMate never reads native accounts or starts login on navigation', async () => {
  await mount(false);
  await act(async () => {
    await state.dumateAccounts.add();
  });
  expect(calls('list_dumate_accounts')).toHaveLength(0);
  expect(calls('start_dumate_login')).toHaveLength(0);
});

it('router navigation only reads saved DuMate accounts, and explicit reauthorization uses the shared login lifecycle', async () => {
  await mount();
  act(() => useNavigationStore.getState().setActivePage('freeModels'));
  await tick();
  expect(calls('list_dumate_accounts')).toHaveLength(2);
  expect(calls('start_dumate_login')).toHaveLength(0);
  expect(calls('refresh_dumate_account_quota')).toHaveLength(0);
  expect(calls('switch_dumate_account')).toHaveLength(0);
  expect(accountProxyStatus(state, 'dumate').accounts[0].id).toBe('saved');
  handlers.set('start_dumate_login', () => ({
    loginId: 'login',
    expiresAt: Date.now() / 1000 + 60,
  }));
  handlers.set('poll_dumate_login', () => row);
  handlers.set('cancel_dumate_login', () => undefined);
  await act(async () => {
    await accountProxyStatus(state, 'dumate').add();
  });
  expect(calls('start_dumate_login')).toHaveLength(1);
  expect(calls('poll_dumate_login')).toEqual([['poll_dumate_login', { loginId: 'login' }]]);
  expect(calls('switch_dumate_account')).toHaveLength(0);
  expect(calls('start_tool')).toHaveLength(0);
});
it('failed or late account switching cannot launch the wrong application', async () => {
  await mount();
  handlers.set('switch_dumate_account', () => {
    throw new Error('accountError.write');
  });
  await act(async () => {
    await state.handleLaunch();
  });
  expect(state.applyError).toBe('accountError.write');
  expect(calls('start_tool')).toHaveLength(0);
  let finish!: (value: DuMateAccount) => void;
  handlers.set(
    'switch_dumate_account',
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  let operation!: Promise<void>;
  await act(async () => {
    operation = state.handleLaunch();
  });
  act(() => state.setSelectedTool(null));
  await act(async () => {
    finish(row);
    await operation;
  });
  expect(calls('start_tool')).toHaveLength(0);
  expect(state.isLaunching).toBe(false);
});
