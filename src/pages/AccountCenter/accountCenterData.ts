import type { AppManagerContextType } from '../AppManager/context';
import type { ManagedAccount } from '../AppManager/useManagedAccounts';
import type { AntigravityQuota, TraeAccount, WorkBuddyAccount } from '../../api/tauri';
import type { TKey } from '../../i18n';
import { codexPlanLabel } from '../AppManager/codexPlanLabel';
import { isFreePlan } from '../AppManager/accountPlanLabel';
import { traeClaimedToday, traeClaimUnavailable } from '../AppManager/traeDailyCredits';

export interface AccountMetric {
  label: string;
  value: string;
  percent?: number | null;
  resetAt?: number | null;
  resetLabel?: string;
  showProgress?: boolean;
}

export interface AccountCardData {
  id: string;
  identity: string;
  detail?: string;
  plan?: string | null;
  subscriptionEndAt?: number | null;
  metrics: AccountMetric[];
  refreshing: boolean;
  authorizationFailed: boolean;
  refresh: (onError?: (error: unknown) => void) => Promise<void>;
  claim?: () => Promise<unknown>;
  claimDisabled?: boolean;
  dailyClaimedAt?: number | null;
  showUnclaimed?: boolean;
  remove: () => void;
}

export interface AccountProvider {
  id: string;
  name: string;
  icon: string;
  website?: string;
  installed: boolean;
  busy: boolean;
  loading: boolean;
  remainingSeconds: number;
  waitingLabel?: string;
  add: () => void;
  cancelLogin: () => void;
  accounts: AccountCardData[];
}

interface AccountGroup<A> {
  accounts: A[];
  busy: boolean;
  loading: boolean;
  remainingSeconds: number;
  awaitingClientExit?: boolean;
  refreshing: Set<string>;
  authorizationFailedIds: Set<string>;
  add: () => Promise<void>;
  cancelLogin: () => void;
  refresh: (
    account: A,
    operation?: (account: A) => Promise<A>,
    onError?: (error: unknown) => void
  ) => Promise<unknown>;
  remove: (account: A) => Promise<void>;
}

export function accountCenterProviders(
  context: AppManagerContextType,
  t: (key: TKey) => string,
  locale: string
): AccountProvider[] {
  const quota = (
    percent?: number | null,
    resetAt?: number | null,
    label = t('accountCenter.quota')
  ): AccountMetric => ({
    label,
    value: percent == null ? '—' : `${Math.round(percent)}%`,
    percent,
    resetAt,
    showProgress: true,
  });
  const number = (value?: number | null) =>
    value == null ? '—' : new Intl.NumberFormat(locale, { maximumFractionDigits: 2 }).format(value);
  const windowLabel = (index: number) =>
    t('accountCenter.window').replace('{n}', String(index + 1));
  const minimum = (quotas: AntigravityQuota[], prefix: string) =>
    quotas
      .filter((q) => q.name.startsWith(prefix))
      .reduce<
        AntigravityQuota | undefined
      >((lowest, q) => (!lowest || q.remainingPercent < lowest.remainingPercent ? q : lowest), undefined);
  const credits = (
    label: string,
    remaining?: number | null,
    total?: number | null,
    resetAt?: number | null,
    currency?: string | null
  ): AccountMetric => ({
    label,
    value: remaining != null && currency === 'USD' ? `$${number(remaining)}` : number(remaining),
    resetAt,
    percent:
      remaining != null && total != null && total > 0 ? (remaining / total) * 100 : undefined,
  });
  const creditMetrics = (account: WorkBuddyAccount | TraeAccount): AccountMetric[] => {
    const metrics: AccountMetric[] = [];
    if (account.baseRemaining != null)
      metrics.push(
        credits(
          t('agent.baseCredits'),
          account.baseRemaining,
          account.baseTotal,
          account.baseResetAt
        )
      );
    if (account.rewardRemaining != null)
      metrics.push(credits(t('agent.rewardCredits'), account.rewardRemaining, account.rewardTotal));
    if (account.addonRemaining != null)
      metrics.push(credits(t('agent.purchasedCredits'), account.addonRemaining));
    return metrics.length
      ? metrics
      : [credits(t('agent.credits'), account.remaining, account.total, account.expiresAt)];
  };
  const provider = <A extends ManagedAccount>(
    id: string,
    name: string,
    icon: string,
    toolIds: string[],
    group: AccountGroup<A>,
    describe: (
      account: A
    ) => Pick<
      AccountCardData,
      | 'identity'
      | 'detail'
      | 'plan'
      | 'metrics'
      | 'subscriptionEndAt'
      | 'claim'
      | 'claimDisabled'
      | 'dailyClaimedAt'
      | 'showUnclaimed'
    >
  ): AccountProvider => ({
    id,
    name,
    icon,
    website: context.detectedTools.find((tool) => toolIds.includes(tool.id) && tool.website)
      ?.website,
    installed: context.detectedTools.some((tool) => toolIds.includes(tool.id) && tool.installed),
    busy: group.busy,
    loading: group.loading,
    remainingSeconds: group.remainingSeconds,
    waitingLabel:
      id === 'cue' && group.busy
        ? t(group.awaitingClientExit ? 'agent.cueExitClient' : 'agent.cueLoginClient')
        : id === 'claudedesktop' && group.busy
          ? t(
              group.awaitingClientExit
                ? 'agent.claudeDesktopExitClient'
                : 'agent.claudeDesktopLoginClient'
            )
          : group.awaitingClientExit
            ? t('agent.manusExitClient')
            : undefined,
    add: () => void group.add(),
    cancelLogin: group.cancelLogin,
    accounts: group.accounts.map((account) => {
      const summary = describe(account);
      const free = isFreePlan(summary.plan);
      return {
        id: account.id,
        ...summary,
        subscriptionEndAt:
          free && summary.subscriptionEndAt == null ? undefined : summary.subscriptionEndAt,
        metrics:
          id !== 'trae' &&
          id !== 'traecn' &&
          free &&
          summary.metrics.every(
            (metric) => metric.value === '—' && metric.percent == null && metric.resetAt == null
          )
            ? []
            : summary.metrics,
        refreshing: group.refreshing.has(account.id),
        authorizationFailed: group.authorizationFailedIds.has(account.id),
        refresh: async (onError) => {
          await group.refresh(account, undefined, onError);
        },
        remove: () => void group.remove(account),
      };
    }),
  });
  return [
    provider(
      'codex',
      'ChatGPT / Codex',
      '/icons/tools/codex.svg',
      ['chatgptdesktop', 'codex'],
      {
        accounts: context.codexAccounts,
        busy: context.isAddingCodexAccount,
        loading: context.isLoadingCodexAccounts,
        remainingSeconds: context.codexOAuthRemainingSeconds,
        refreshing: context.refreshingCodexAccountIds,
        authorizationFailedIds: context.codexAuthorizationFailedIds,
        add: context.addCodexAccount,
        cancelLogin: context.cancelCodexLogin,
        refresh: context.refreshCodexAccountQuota,
        remove: context.deleteCodexAccount,
      },
      (account) => {
        const plan = codexPlanLabel(account.plan);
        const windows = account.quotaWindows ?? [];
        const metrics =
          windows.length && windows.every((w) => w.label === '5h' || w.label === '7d')
            ? (plan === 'Plus' ? ['5h', '7d'] : ['7d']).map((label) => {
                const window = windows.find((w) => w.label === label);
                return quota(window?.remainingPercent, window?.resetAt, label);
              })
            : windows.length
              ? windows
                  .filter((w) => w.label !== '5h' || plan === 'Plus')
                  .map((w, i) => quota(w.remainingPercent, w.resetAt, w.label || windowLabel(i)))
              : [quota(account.quotaPercent, account.quotaResetAt)];
        return {
          identity: account.email,
          plan,
          subscriptionEndAt:
            account.plan?.trim().toLowerCase() === 'free'
              ? undefined
              : (account.subscriptionEndAt ?? null),
          metrics,
        };
      }
    ),
    provider(
      'claudecode',
      'Claude Code',
      '/icons/tools/claudecode.svg',
      ['claudecode'],
      context.claudeCodeAccounts,
      (account) => ({
        identity: account.email,
        plan: account.plan,
        metrics: [
          quota(account.fiveHour?.remainingPercent, account.fiveHour?.resetAt, '5h'),
          quota(account.sevenDay?.remainingPercent, account.sevenDay?.resetAt, '7d'),
        ],
      })
    ),
    provider(
      'claudedesktop',
      context.detectedTools.find((tool) => tool.id === 'claudedesktop')?.name ?? 'Claude Desktop',
      '/icons/tools/claudedesktop.svg',
      ['claudedesktop'],
      context.claudeDesktopAccounts,
      (account) => ({
        identity: account.email,
        plan: account.plan,
        metrics: [
          quota(account.fiveHour?.remainingPercent, account.fiveHour?.resetAt, '5h'),
          quota(account.sevenDay?.remainingPercent, account.sevenDay?.resetAt, '7d'),
        ],
      })
    ),
    provider(
      'dsh',
      'DeepSeek Harness',
      '/icons/tools/dsh.png',
      ['dsh'],
      context.deepSeekAccounts,
      (account) => ({
        identity: account.name,
        metrics: account.balances?.length
          ? account.balances.map((balance) => ({
              label: t('model.balance'),
              value: new Intl.NumberFormat(locale, {
                style: 'currency',
                currency: balance.currency,
                maximumFractionDigits: 2,
              }).format(balance.amount),
            }))
          : [{ label: t('model.balance'), value: account.balances == null ? '—' : '0' }],
      })
    ),
    ...(['workbuddy', 'workbuddyai'] as const).map((edition) =>
      provider(
        edition,
        edition === 'workbuddy' ? 'WorkBuddy' : 'WorkBuddy AI',
        `/icons/tools/${edition}.png`,
        [edition],
        context.workBuddyAccountGroups[edition],
        (account) => ({
          identity: account.name,
          plan: account.plan,
          metrics: creditMetrics(account),
          ...(edition === 'workbuddy'
            ? {
                claim: () => context.workBuddyAccountGroups.workbuddy.claimDaily(account),
                dailyClaimedAt: account.dailyClaimedAt,
              }
            : {}),
        })
      )
    ),
    ...(['trae', 'traecn'] as const).map((edition) =>
      provider(
        edition,
        edition === 'trae' ? 'TRAE' : 'TRAE CN',
        `/icons/tools/${edition}.svg`,
        [edition],
        context.traeAccountGroups[edition],
        (account) => {
          const metrics =
            edition === 'traecn'
              ? creditMetrics(account)
              : [
                  credits(
                    t('accountCenter.quota'),
                    account.remaining,
                    account.total,
                    account.expiresAt,
                    account.currency
                  ),
                ];
          if (edition === 'traecn') {
            for (const metric of metrics) {
              if (
                metric.label === t('agent.baseCredits') &&
                account.baseRemaining === 0 &&
                account.baseTotal === 0
              )
                metric.percent = 0;
              if (metric.label === t('agent.rewardCredits') && account.rewardNextExpiryAt != null) {
                metric.resetAt = account.rewardNextExpiryAt;
                metric.resetLabel = t('agent.rewardCreditsNextExpiry');
              }
            }
          }
          return {
            identity: account.name,
            plan: account.plan,
            subscriptionEndAt: account.subscriptionEndAt ?? undefined,
            metrics,
            ...(edition === 'traecn'
              ? {
                  claim: async () => {
                    const group = context.traeAccountGroups.traecn;
                    if (traeClaimUnavailable(account, group.accounts)) return;
                    const updated = await group.claimDaily(account);
                    return updated != null && traeClaimedToday(updated.deviceClaimedAt);
                  },
                  claimDisabled:
                    context.traeAccountGroups.traecn.refreshing.size > 0 ||
                    traeClaimUnavailable(account, context.traeAccountGroups.traecn.accounts),
                  dailyClaimedAt: account.dailyClaimedAt,
                  showUnclaimed: account.rewardEnabled === true,
                }
              : {}),
          };
        }
      )
    ),
    provider(
      'antigravity',
      'Antigravity',
      '/icons/tools/antigravitydesktop.png',
      ['antigravitydesktop', 'antigravity'],
      context.antigravityAccounts,
      (account) => ({
        identity: account.email,
        plan: account.plan,
        metrics: ['gemini', 'claude'].map((family) => {
          const q = minimum(account.quotas, family);
          return quota(q?.remainingPercent, q?.resetAt, family === 'gemini' ? 'Gemini' : 'Claude');
        }),
      })
    ),
    provider(
      'cursor',
      'Cursor',
      '/icons/tools/cursor.svg',
      ['cursor'],
      context.cursorAccounts,
      (account) => ({
        identity: account.email,
        plan: account.usage?.plan,
        metrics: [quota(account.usage?.remainingPercent, account.usage?.resetAt)],
      })
    ),
    {
      ...provider(
        'zcode',
        'ZCode',
        '/icons/tools/zcode.png',
        ['zcode'],
        context.zcodeAccounts,
        (account) => ({
          identity: account.email,
          detail: account.provider === 'bigmodel' ? 'BigModel' : 'Z.ai',
          plan:
            account.plan === 'Trial'
              ? t('agent.zcodeTrial')
              : account.plan?.replace(/^ZCode\s+/i, ''),
          subscriptionEndAt:
            ['trial', 'free'].includes(account.plan?.toLowerCase() ?? '') ||
            (!account.plan && account.subscriptionEndAt == null)
              ? undefined
              : (account.subscriptionEndAt ?? null),
          metrics: account.quotaWindows?.length
            ? account.quotaWindows.map((w, i) =>
                quota(w.remainingPercent, w.resetAt, windowLabel(i))
              )
            : [quota(account.remainingPercent, account.resetAt)],
        })
      ),
      website:
        context.zcodeAccounts.provider === 'bigmodel'
          ? 'https://bigmodel.cn/glm-coding'
          : 'https://z.ai/subscribe',
    },
    provider(
      'grok',
      'Grok Build',
      '/icons/tools/grok.svg',
      ['grok'],
      context.grokAccounts,
      (account) => ({ identity: account.email, plan: account.plan, metrics: [] })
    ),
    provider(
      'grokbot',
      'Grok Bot',
      '/icons/tools/grokbot.png',
      ['grokbot'],
      context.grokBotAccounts,
      (account) => ({
        identity: account.email,
        plan:
          account.usage?.plan?.trim().toLowerCase() === 'grok bot plan'
            ? null
            : account.usage?.plan,
        metrics: [quota(account.usage?.remainingPercent, account.usage?.resetAt)],
      })
    ),
    ...(['manus', 'cue'] as const).map((tool) =>
      provider(
        tool,
        tool === 'cue' ? 'Cue' : 'Manus',
        `/icons/tools/${tool}.png`,
        [tool],
        tool === 'cue' ? context.cueAccounts : context.manusAccounts,
        (account) => ({
          identity: account.email,
          plan: account.plan,
          subscriptionEndAt:
            !isFreePlan(account.plan) && 'subscriptionEndAt' in account
              ? (account.subscriptionEndAt ?? undefined)
              : undefined,
          metrics:
            tool === 'cue' && 'weekly' in account && account.weekly
              ? [quota(account.weekly.remainingPercent, account.weekly.resetAt, '7d')]
              : [
                  {
                    label: t('agent.credits'),
                    value: number('credits' in account ? account.credits?.total : null),
                  },
                ],
        })
      )
    ),
  ];
}
