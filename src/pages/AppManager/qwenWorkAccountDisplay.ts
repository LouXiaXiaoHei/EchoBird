import type { QwenWorkAccount } from '../../api/qwenwork';
import type { TKey } from '../../i18n';

export function qwenWorkIdentity(
  account: Pick<QwenWorkAccount, 'name' | 'email' | 'id' | 'organization'>
): string {
  return [account.name || account.email || account.id, account.organization]
    .filter(Boolean)
    .join(' · ');
}

export function qwenWorkPlanLabel(
  account: Pick<QwenWorkAccount, 'edition' | 'plan' | 'planName'>,
  t: (key: TKey) => string
): string {
  const plan = account.plan?.trim().toLowerCase();
  const english = {
    'agent.qwenworkFree': 'Free',
    'agent.qwenworkPlus': 'Plus',
    'agent.qwenworkPro': 'Pro',
    'agent.qwenworkEnterpriseStandard': 'Enterprise Standard',
    'agent.qwenworkEnterprisePremium': 'Enterprise Premium',
  } as const;
  const labels: Record<string, keyof typeof english> = {
    free: 'agent.qwenworkFree',
    personal_free: 'agent.qwenworkFree',
    'qwen-office-free': 'agent.qwenworkFree',
    plus: 'agent.qwenworkPlus',
    personal_standard: 'agent.qwenworkPlus',
    'qwen-office-plus': 'agent.qwenworkPlus',
    'qwen-office-personal': 'agent.qwenworkPlus',
    pro: 'agent.qwenworkPro',
    personal_premium: 'agent.qwenworkPro',
    'qwen-office-pro': 'agent.qwenworkPro',
    'qwen-office-flagship': 'agent.qwenworkPro',
    enterprise_standard: 'agent.qwenworkEnterpriseStandard',
    'qwen-office-enterprise-standard': 'agent.qwenworkEnterpriseStandard',
    enterprise_premium: 'agent.qwenworkEnterprisePremium',
    'qwen-office-enterprise-premium': 'agent.qwenworkEnterprisePremium',
    'enterprise standard': 'agent.qwenworkEnterpriseStandard',
    'enterprise premium': 'agent.qwenworkEnterprisePremium',
    免费版: 'agent.qwenworkFree',
    个人标准版: 'agent.qwenworkPlus',
    个人高级版: 'agent.qwenworkPro',
    企业标准版: 'agent.qwenworkEnterpriseStandard',
    企业旗舰版: 'agent.qwenworkEnterprisePremium',
  };
  const key = [plan, account.planName?.trim().toLowerCase()].find(
    (value) => value && Object.prototype.hasOwnProperty.call(labels, value)
  );
  return key
    ? account.edition === 'qwenwork'
      ? english[labels[key]]
      : t(labels[key])
    : account.planName || account.plan || '';
}

export function qwenWorkCreditBalances(
  account: Pick<QwenWorkAccount, 'edition' | 'remaining' | 'total' | 'creditBuckets'>
) {
  if (account.edition === 'qwenwork') {
    return { package: { remaining: account.remaining, total: account.total }, reward: undefined };
  }
  const daily = account.creditBuckets.find((bucket) => bucket.name === 'Daily');
  const monthly = account.creditBuckets.find((bucket) => bucket.name === 'Monthly');
  const longterm = account.creditBuckets.find((bucket) => bucket.name === 'Long-term');
  const difference = (total: number | null, reward: number | null) =>
    total != null && reward != null && total >= reward ? total - reward : null;
  return {
    package: {
      remaining:
        monthly?.remaining != null && longterm?.remaining != null
          ? monthly.remaining + longterm.remaining
          : daily
            ? difference(account.remaining, daily.remaining)
            : account.remaining,
      total:
        monthly?.total != null && longterm?.total != null
          ? monthly.total + longterm.total
          : daily
            ? difference(account.total, daily.total)
            : account.total,
    },
    reward: {
      remaining: daily?.remaining ?? null,
      total: daily?.total ?? null,
      expiresAt: daily?.expiresAt,
    },
  };
}
