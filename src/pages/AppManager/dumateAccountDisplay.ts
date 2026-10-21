import type { DuMateAccount } from '../../api/dumate';
import type { TKey } from '../../i18n';

export function dumatePlanLabel(account: DuMateAccount, t: (key: TKey) => string) {
  switch (account.plan) {
    case 'free':
      return t('agent.qwenworkFree');
    case 'pro':
      return 'Pro';
    case 'max':
      return 'Max';
    case 'team':
      return t('agent.dumateEnterpriseSeat');
    case 'experience':
      return t('agent.dumateTrial');
    default:
      return account.plan ?? '';
  }
}
export function dumateCreditBalances(account: DuMateAccount) {
  const bucket = (reward: boolean) => {
    const rows = account.creditBuckets.filter((b) => b.reward === reward);
    const sum = (key: 'remaining' | 'total') =>
      rows.some((b) => b[key] == null) ? null : rows.reduce((sum, b) => sum + b[key]!, 0);
    const known = account.remaining != null && account.total != null;
    return {
      remaining: rows.length || known ? sum('remaining') : null,
      total: rows.length || known ? sum('total') : null,
      expiresAt: rows.reduce<number | undefined>(
        (first, b) =>
          b.expiresAt != null && (first == null || b.expiresAt < first) ? b.expiresAt : first,
        undefined
      ),
    };
  };
  return { package: bucket(false), reward: bucket(true) };
}
export function dumateRewardStatus(account: DuMateAccount, now = Date.now() / 1000) {
  if (account.enterprise) return 'unavailable';
  const reward = account.dailyReward;
  if (
    !reward ||
    !Number.isFinite(reward.checkedAt) ||
    reward.checkedAt <= 0 ||
    reward.checkedAt > now ||
    now >= reward.nextClaimAt
  )
    return 'unknown';
  return reward.status === 'CLAIMED_TODAY' ? 'claimed' : 'claimable';
}
