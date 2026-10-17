import type { CodeArtsAccount } from '../../api/codeArtsAccounts';

export function codeArtsRemainingPercent(account: CodeArtsAccount): number | undefined {
  if (
    account.balance == null ||
    !Number.isFinite(account.balance) ||
    account.balance < 0 ||
    account.total == null ||
    !Number.isFinite(account.total) ||
    account.total <= 0
  )
    return undefined;
  return Math.round(Math.min(100, (account.balance / account.total) * 100) * 100) / 100;
}
