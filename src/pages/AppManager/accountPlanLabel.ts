import type { ProxyAccount } from '../../api/accountProxy';
import { codexPlanLabel } from './codexPlanLabel';

export function isFreePlan(plan?: string | null): boolean {
  return plan?.trim().toLowerCase() === 'free';
}

export function accountPlanLabel(plan: string): string {
  return isFreePlan(plan) ? 'Free' : plan.replace(/^[a-z]/, (letter) => letter.toUpperCase());
}
export function accountProxyPlanLabel(account: ProxyAccount): string {
  return account.source.provider === 'codex'
    ? codexPlanLabel(account.plan)
    : account.plan
      ? accountPlanLabel(account.plan)
      : '';
}
