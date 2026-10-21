import { invoke } from '@tauri-apps/api/core';

export interface DuMateAccount {
  id: string;
  name: string;
  enterprise: boolean;
  plan: string | null;
  expiresAt: number | null;
  active: boolean;
  remaining: number | null;
  total: number | null;
  creditBuckets: {
    name: string;
    source: string;
    reward: boolean;
    remaining: number | null;
    total: number | null;
    expiresAt: number | null;
  }[];
  dailyReward: {
    status: 'CLAIMABLE' | 'CLAIMED_TODAY';
    checkedAt: number;
    nextClaimAt: number;
  } | null;
}
export interface DuMateLogin {
  loginId: string;
  expiresAt: number;
}
export const listDuMateAccounts = () => invoke<DuMateAccount[]>('list_dumate_accounts');
export const startDuMateLogin = () => invoke<DuMateLogin>('start_dumate_login');
export const pollDuMateLogin = (loginId: string) =>
  invoke<DuMateAccount | null>('poll_dumate_login', { loginId });
export const cancelDuMateLogin = (loginId: string) =>
  invoke<void>('cancel_dumate_login', { loginId });
export const refreshDuMateAccountQuota = (accountId: string) =>
  invoke<DuMateAccount>('refresh_dumate_account_quota', { accountId });
export const claimDuMateDailyCredits = (accountId: string) =>
  invoke<DuMateAccount>('claim_dumate_daily_credits', { accountId });
export const deleteDuMateAccount = (accountId: string) =>
  invoke<void>('delete_dumate_account', { accountId });
export const switchDuMateAccount = (accountId: string) =>
  invoke<DuMateAccount>('switch_dumate_account', { accountId });
