import { invoke } from '@tauri-apps/api/core';

export type QwenWorkEdition = 'qwenwork' | 'qwenworkcn';
export interface QwenWorkAccount {
  id: string;
  edition: QwenWorkEdition;
  name: string;
  email: string | null;
  organization: string | null;
  plan: string | null;
  planName: string | null;
  expiresAt: number | null;
  active: boolean;
  remaining: number | null;
  total: number | null;
  creditBuckets: {
    name: string;
    remaining: number | null;
    total: number | null;
    expiresAt: number | null;
  }[];
  dailyReward: {
    status: 'CLAIMABLE' | 'CLAIMED_TODAY' | 'DISABLED' | 'NOT_STARTED' | 'ENDED';
    rewardCredits: number | null;
    checkedAt: number;
    nextClaimAt: number | null;
    expiresAt: number | null;
  } | null;
}
export interface QwenWorkLogin {
  loginId: string;
  verificationUri: string;
  expiresAt: number;
}
export const listQwenWorkAccounts = (edition: QwenWorkEdition) =>
  invoke<QwenWorkAccount[]>('list_qwenwork_accounts', { edition });
export const startQwenWorkLogin = (edition: QwenWorkEdition) =>
  invoke<QwenWorkLogin>('start_qwenwork_login', { edition });
export const pollQwenWorkLogin = (loginId: string) =>
  invoke<QwenWorkAccount | null>('poll_qwenwork_login', { loginId });
export const cancelQwenWorkLogin = (loginId: string) =>
  invoke<void>('cancel_qwenwork_login', { loginId });
export const refreshQwenWorkAccountQuota = (edition: QwenWorkEdition, accountId: string) =>
  invoke<QwenWorkAccount>('refresh_qwenwork_account_quota', { edition, accountId });
export const deleteQwenWorkAccount = (edition: QwenWorkEdition, accountId: string) =>
  invoke<void>('delete_qwenwork_account', { edition, accountId });
export const switchQwenWorkAccount = (edition: QwenWorkEdition, accountId: string) =>
  invoke<QwenWorkAccount>('switch_qwenwork_account', { edition, accountId });
