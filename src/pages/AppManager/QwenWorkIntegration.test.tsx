import { useLayoutEffect } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { AppManagerProvider } from './AppManagerProvider';
import { useAppManager } from './context';
import { QwenWorkAccountSection } from './QwenWorkAccountSection';
import { AccountSectionRow, AccountSectionRewardButton } from './AccountSectionPrimitives';
import { useToolsStore } from '../../stores/toolsStore';
import { useNavigationStore } from '../../stores/navigationStore';
import type { QwenWorkAccount, QwenWorkEdition } from '../../api/qwenwork';
import cnDefinition from '../../../tools/qwenworkcn/paths.json';
import globalDefinition from '../../../tools/qwenwork/paths.json';

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

let state: ReturnType<typeof useAppManager>;
let renderer: ReactTestRenderer;
const account = (edition: QwenWorkEdition): QwenWorkAccount => ({
  id: edition,
  edition,
  name: edition,
  email: null,
  organization: 'Fixture team',
  plan: 'enterprise_premium',
  planName: 'Enterprise Premium',
  expiresAt: null,
  active: true,
  remaining: null,
  total: null,
  creditBuckets: [],
  dailyReward: null,
});
function Harness() {
  const context = useAppManager();
  useLayoutEffect(() => {
    state = context;
  });
  return <QwenWorkAccountSection />;
}
const calls = (name: string) => invoke.mock.calls.filter(([command]) => command === name);
async function tick() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
}
async function mount(edition: QwenWorkEdition, installed = true) {
  useToolsStore.getState().setDetectedTools([
    { ...cnDefinition, id: 'qwenworkcn', installed: installed && edition === 'qwenworkcn' },
    { ...globalDefinition, id: 'qwenwork', installed: installed && edition === 'qwenwork' },
  ]);
  act(() => {
    renderer = create(
      <AppManagerProvider>
        <Harness />
      </AppManagerProvider>
    );
  });
  act(() => {
    state.setSelectedTool(edition);
  });
  await tick();
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  useNavigationStore.setState({ activePage: 'apps' });
  const storage = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  });
  invoke.mockImplementation(async (command: string, args?: { edition?: QwenWorkEdition }) => {
    if (command === 'get_models') return [];
    if (command === 'get_tool_install_index') return '{"ids":[]}';
    if (command === 'list_qwenwork_accounts') return [account(args!.edition!)];
    if (command === 'refresh_qwenwork_account_quota')
      return { ...account(args!.edition!), remaining: 127 };
    if (command === 'switch_qwenwork_account') return account(args!.edition!);
    return [];
  });
});
afterEach(() => {
  act(() => {
    renderer?.unmount();
  });
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it.each(['qwenworkcn', 'qwenwork'] as const)(
  '%s uses the installed edition and applies its account before launch, without models or check-in',
  async (edition) => {
    await mount(edition);
    expect(calls('list_qwenwork_accounts')).toHaveLength(1);
    expect(calls('list_qwenwork_accounts')[0][1]).toEqual({ edition });
    expect(state.qwenWorkAccounts.selectedId).toBe(edition);
    expect(renderer.root.findAllByType(AccountSectionRewardButton)).toHaveLength(0);
    if (edition === 'qwenwork')
      expect(renderer.root.findByType(AccountSectionRow).props.plan).toBe('Enterprise Premium');
    expect(calls('refresh_qwenwork_account_quota')).toHaveLength(0);
    expect(calls('claim_qwenwork_daily_credits')).toHaveLength(0);
    await act(async () => {
      await state.handleLaunch();
    });
    expect(calls('switch_qwenwork_account')[0][1]).toEqual({ edition, accountId: edition });
    expect(calls('start_tool')).toHaveLength(1);
    const commands = invoke.mock.calls.map(([command]) => command);
    expect(commands.indexOf('switch_qwenwork_account')).toBeLessThan(
      commands.indexOf('start_tool')
    );
    expect(calls('apply_model_to_tool')).toHaveLength(0);
    expect(calls('claim_qwenwork_daily_credits')).toHaveLength(0);
  }
);

it('nested refresh keeps selection separate and uses the shared row controls', async () => {
  await mount('qwenworkcn');
  act(() => {
    state.qwenWorkAccounts.select(null);
  });
  const row = renderer.root.findByType(AccountSectionRow);
  const refresh = renderer.root
    .findAllByType('button')
    .find((button) => /^agent.refreshAccount/.test(button.props['aria-label'] ?? ''))!;
  const stopPropagation = vi.fn();
  await act(async () => {
    refresh.props.onClick({ stopPropagation });
  });
  expect(stopPropagation).toHaveBeenCalledOnce();
  expect(state.qwenWorkAccounts.selectedId).toBeNull();
  expect(calls('refresh_qwenwork_account_quota')).toHaveLength(1);
  act(() => {
    row.props.onSelect();
  });
  expect(state.qwenWorkAccounts.selectedId).toBe('qwenworkcn');
  expect(calls('switch_qwenwork_account')).toHaveLength(0);
  expect(calls('claim_qwenwork_daily_credits')).toHaveLength(0);
});

it.each(['CLAIMABLE', 'CLAIMED_TODAY', null] as const)(
  'shows daily balance without claim controls even with cached reward status %s',
  async (status) => {
    const row: QwenWorkAccount = {
      ...account('qwenworkcn'),
      remaining: 2099.75,
      creditBuckets: [
        { name: 'Daily', remaining: 99.75, total: null, expiresAt: null },
        { name: 'Monthly', remaining: 0, total: null, expiresAt: null },
        { name: 'Long-term', remaining: 2000, total: null, expiresAt: null },
      ],
      dailyReward: status
        ? {
            status,
            rewardCredits: 100,
            checkedAt: Date.now() / 1000,
            nextClaimAt: Date.now() / 1000 + 3600,
            expiresAt: null,
          }
        : null,
    };
    invoke.mockImplementation(async (command: string) =>
      command === 'list_qwenwork_accounts' ? [row] : []
    );
    await mount('qwenworkcn');
    const base = renderer.root.findByProps({ 'aria-label': 'agent.baseCredits' });
    expect(base.children).toEqual(['2000']);
    expect(renderer.root.findByProps({ 'aria-label': 'agent.rewardCredits' }).children).toContain(
      '99.75'
    );
    expect(
      renderer.root.findAllByProps({ 'aria-label': 'agent.dailyCreditsUnclaimed' })
    ).toHaveLength(0);
    expect(renderer.root.findAllByType(AccountSectionRewardButton)).toHaveLength(0);
    expect(calls('refresh_qwenwork_account_quota')).toHaveLength(0);
    expect(calls('claim_qwenwork_daily_credits')).toHaveLength(0);
  }
);

it('an uninstalled client does not list credentials or begin login', async () => {
  await mount('qwenworkcn', false);
  await act(async () => {
    await state.qwenWorkAccounts.add();
  });
  expect(calls('list_qwenwork_accounts')).toHaveLength(0);
  expect(calls('start_qwenwork_login')).toHaveLength(0);
});

it('a pending switch remains busy beyond the old cooldown and leaving prevents late launch', async () => {
  await mount('qwenwork');
  let resolve!: (value: unknown) => void;
  invoke.mockImplementation((command: string) =>
    command === 'switch_qwenwork_account'
      ? new Promise((done) => {
          resolve = done;
        })
      : Promise.resolve([])
  );
  let pending!: Promise<void>;
  act(() => {
    pending = state.handleLaunch();
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(state.isLaunching).toBe(true);
  act(() => {
    useNavigationStore.getState().setActivePage('models');
  });
  await act(async () => {
    resolve(account('qwenwork'));
    await pending;
  });
  expect(calls('start_tool')).toHaveLength(0);
  expect(state.applyError).toBeNull();
});

it.each([
  ['qwenworkcn', 'page'],
  ['qwenworkcn', 'tool'],
  ['qwenwork', 'page'],
  ['qwenwork', 'tool'],
] as const)(
  '%s does not launch after leaving the %s during the post-switch list read',
  async (edition, exit) => {
    await mount(edition);
    let resolveList!: (rows: QwenWorkAccount[]) => void;
    invoke.mockImplementation((command: string) =>
      command === 'list_qwenwork_accounts'
        ? new Promise((resolve) => {
            resolveList = resolve;
          })
        : Promise.resolve(account(edition))
    );
    let pending!: Promise<void>;
    await act(async () => {
      pending = state.handleLaunch();
    });
    expect(resolveList).toBeTypeOf('function');
    act(() => {
      if (exit === 'page') useNavigationStore.getState().setActivePage('models');
      else state.setSelectedTool('workbuddy');
    });
    await act(async () => {
      resolveList([account(edition)]);
      await pending;
    });
    expect(calls('switch_qwenwork_account')).toHaveLength(1);
    expect(calls('start_tool')).toHaveLength(0);
    expect(state.applyError).toBeNull();
  }
);
