import { invoke } from '@tauri-apps/api/core';

export type DoubaoEdition = 'doubaowork' | 'dola';

export interface DoubaoQuotaWindow {
  kind: string;
  label: string | null;
  remaining: number | null;
  total: number | null;
  usedPercent: number | null;
  resetAt: number | null;
  unlimited: boolean;
}

export interface DoubaoQuota {
  windows: DoubaoQuotaWindow[];
  bonus: {
    remaining: number | null;
    total: number | null;
    usedPercent: number | null;
    expiresAt: number | null;
  } | null;
}

export interface DoubaoDailyReward {
  status: 'available' | 'claimed' | 'unavailable' | 'unknown';
  scene: string | null;
  nextClaimAt: number | null;
}

export interface DoubaoAccount {
  id: string;
  edition: DoubaoEdition;
  name: string;
  email: string | null;
  plan: string | null;
  expiresAt: number | null;
  remaining: number | null;
  total: number | null;
  quota?: DoubaoQuota | null;
  dailyReward?: DoubaoDailyReward | null;
  active: boolean;
}

export interface DoubaoLogin {
  loginId: string;
  expiresAt: number;
}

export const listDoubaoAccounts = (edition: DoubaoEdition) =>
  invoke<DoubaoAccount[]>('list_doubao_accounts', { edition });
export const startDoubaoLogin = (edition: DoubaoEdition) =>
  invoke<DoubaoLogin>('start_doubao_login', { edition });
export const pollDoubaoLogin = (loginId: string) =>
  invoke<DoubaoAccount | null>('poll_doubao_login', { loginId });
export const cancelDoubaoLogin = (loginId: string) =>
  invoke<void>('cancel_doubao_login', { loginId });
export const refreshDoubaoAccountQuota = (edition: DoubaoEdition, accountId: string) =>
  invoke<DoubaoAccount>('refresh_doubao_account_quota', { edition, accountId });
export const deleteDoubaoAccount = (edition: DoubaoEdition, accountId: string) =>
  invoke<void>('delete_doubao_account', { edition, accountId });
export const switchDoubaoAccount = (edition: DoubaoEdition, accountId: string) =>
  invoke<DoubaoAccount>('switch_doubao_account', { edition, accountId });
export const claimDoubaoDailyCredits = (edition: DoubaoEdition, accountId: string) =>
  invoke<DoubaoAccount>('claim_doubao_daily_credits', { edition, accountId });
