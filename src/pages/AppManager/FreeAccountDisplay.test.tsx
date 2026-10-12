import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, expect, it, vi } from 'vitest';
import type { AppManagerContextType } from './context';
import { AccountCard } from '../AccountCenter/AccountCenter';
import { accountCenterProviders } from '../AccountCenter/accountCenterData';
import { AntigravityAccountSection } from './AntigravityAccountSection';
import { CodexAccountSection } from './AppManagerComponents';
import { ClaudeCodeAccountSection } from './ClaudeCodeAccountSection';
import { CursorAccountSection } from './CursorAccountSection';
import { GrokAccountSection } from './GrokAccountSection';
import { ManusAccountSection } from './ManusAccountSection';
import { WorkBuddyAccountSection } from './WorkBuddyAccountSection';
import { ZCodeAccountSection } from './ZCodeAccountSection';

let state: AppManagerContextType;
vi.mock('./context', () => ({ useAppManager: () => state }));
vi.mock('../../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock('../../components', () => ({ EffortPulse: () => null, getModelIcon: () => null }));

const sections = {
  codex: () => <CodexAccountSection />,
  claudecode: () => <ClaudeCodeAccountSection />,
  claudedesktop: () => <ClaudeCodeAccountSection desktop />,
  cursor: () => <CursorAccountSection tool="cursor" />,
  grokbot: () => <CursorAccountSection tool="grokbot" />,
  grok: () => <GrokAccountSection />,
  manus: () => <ManusAccountSection />,
  cue: () => <ManusAccountSection tool="cue" />,
  antigravity: () => <AntigravityAccountSection />,
  workbuddy: () => <WorkBuddyAccountSection />,
  zcode: () => <ZCodeAccountSection />,
};
type Tool = keyof typeof sections;
const row = () => ({
  id: 'saved',
  email: 'free@example.test',
  name: 'Free user',
  active: true,
  plan: 'FREE',
  provider: 'bigmodel',
  edition: 'workbuddy',
  remaining: null,
  total: null,
  remainingPercent: null,
  resetAt: null,
  expiresAt: null,
  quotas: [],
  usage: { plan: 'FREE', remainingPercent: null, resetAt: null },
  credits: null,
});
beforeEach(() => {
  const account = row();
  const group = {
    accounts: [account],
    busy: false,
    loading: false,
    remainingSeconds: 0,
    refreshing: new Set(),
    authorizationFailedIds: new Set(),
    add: vi.fn(),
    select: vi.fn(),
    refresh: vi.fn(),
    remove: vi.fn(),
    claimDaily: vi.fn(),
  };
  state = {
    detectedTools: [],
    isLaunching: false,
    selectedTool: 'workbuddy',
    codexAccounts: [account],
    selectedCodexAccountId: 'saved',
    isAddingCodexAccount: false,
    isLoadingCodexAccounts: false,
    codexOAuthRemainingSeconds: 0,
    refreshingCodexAccountIds: new Set(),
    codexAuthorizationFailedIds: new Set(),
    addCodexAccount: vi.fn(),
    setSelectedCodexAccountId: vi.fn(),
    refreshCodexAccountQuota: vi.fn(),
    deleteCodexAccount: vi.fn(),
    claudeCodeAccounts: group,
    claudeDesktopAccounts: group,
    cursorAccounts: group,
    grokBotAccounts: group,
    grokAccounts: group,
    kiloAccounts: { ...group, accounts: [] },
    copilotAccounts: { ...group, accounts: [] },
    manusAccounts: group,
    cueAccounts: group,
    antigravityAccounts: group,
    workBuddyAccounts: group,
    workBuddyAccountGroups: { workbuddy: group, workbuddyai: group },
    traeAccountGroups: { trae: { ...group, accounts: [] }, traecn: { ...group, accounts: [] } },
    zcodeAccounts: { ...group, provider: 'bigmodel', setProvider: vi.fn() },
    deepSeekAccounts: { ...group, accounts: [] },
  } as unknown as AppManagerContextType;
});
function markup(tool: Tool) {
  const provider = accountCenterProviders(state, (key) => key, 'en').find((p) => p.id === tool)!;
  return [
    renderToStaticMarkup(sections[tool]()),
    renderToStaticMarkup(<AccountCard provider={provider} account={provider.accounts[0]} />),
  ];
}
it.each(Object.keys(sections) as Tool[])(
  '%s: shows Free and the shared missing-usage message in both account views',
  (tool) => {
    for (const html of markup(tool)) {
      expect(html).toContain('>Free<');
      expect(html).toContain('>model.noUsageData<');
      expect(html).not.toContain('>—<');
      expect(html).not.toContain('role="progressbar"');
      expect(html).not.toContain('>0%<');
    }
  }
);
it.each(Object.keys(sections).filter((tool) => tool !== 'grok') as Tool[])(
  '%s: retains genuine zero usage for a Free account',
  (tool) => {
    Object.assign(state.codexAccounts[0], {
      quotaPercent: 0,
      remainingPercent: 0,
      remaining: 0,
      baseRemaining: 0,
      baseTotal: 100,
      fiveHour: { remainingPercent: 0 },
      quotas: [{ name: 'gemini-3', remainingPercent: 0 }],
      usage: { plan: 'FREE', remainingPercent: 0 },
      credits: { total: 0 },
    });
    for (const html of markup(tool)) {
      expect(html).toContain('>Free<');
      expect(html).not.toContain('>model.noUsageData<');
      expect(html.replace(/<[^>]*>/g, ' ')).toMatch(/\b0(?:%|\b)/);
    }
  }
);
it.each([null, 'SuperGrok'])(
  'keeps Grok plan %s distinct from Free when no numerical usage is available',
  (plan) => {
    state.grokAccounts.accounts[0].plan = plan;
    for (const html of markup('grok')) {
      expect(html).toContain('>model.noUsageData<');
      expect(html).not.toContain('>Free<');
      if (plan) expect(html).toContain(`>${plan}<`);
    }
  }
);

it.each(['free', 'plus', 'pro', 'max'])(
  'Cue: displays the actual %s tier and weekly quota in both account views',
  (plan) => {
    state.cueAccounts = {
      ...state.cueAccounts,
      accounts: [
        {
          ...row(),
          plan,
          credits: { total: 20000, free: null, refresh: null, nextRefreshAt: null },
          weekly: { remainingPercent: 0, resetAt: 1900000000 },
        },
      ],
    };
    for (const html of markup('cue')) {
      expect(html).toContain(`>${plan[0].toUpperCase()}${plan.slice(1)}<`);
      expect(html).toContain('>0%<');
      expect(html).toContain('aria-label="7d free@example.test"');
      expect(html).not.toContain('>model.noUsageData<');
      expect(html).not.toContain('20000');
      expect(html).not.toContain('title=');
      expect(html).not.toContain('cursor-');
    }
    expect(state.cueAccounts.refresh).not.toHaveBeenCalled();
    expect(state.cueAccounts.add).not.toHaveBeenCalled();
    expect(markup('manus')[0]).toContain('>model.noUsageData<');
  }
);

it('Cue: does not fabricate weekly usage for an inactive Free window', () => {
  state.cueAccounts = {
    ...state.cueAccounts,
    accounts: [{ ...row(), weekly: { remainingPercent: null, resetAt: null } }],
  };
  for (const html of markup('cue')) {
    expect(html).toContain('>Free<');
    expect(html).toContain('>model.noUsageData<');
    expect(html).not.toContain('role="progressbar"');
    expect(html).not.toContain('>0%<');
  }
});
