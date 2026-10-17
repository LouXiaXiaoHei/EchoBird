import { invoke } from '@tauri-apps/api/core';

export type KimiRegion = 'cn' | 'global';
export interface KimiAccount {
  id: string;
  email: string;
  region: KimiRegion;
  plan?: string | null;
  quotaWindows: { name: string; remainingPercent: number; resetAt?: number | null }[];
  wallet?: { remaining: number; total: number; currency?: string | null } | null;
  active: boolean;
}
export interface KimiLogin {
  loginId: string;
  verificationUri: string;
  expiresAt: number;
  pollIntervalSeconds: number;
}
export const listKimiAccounts = () => invoke<KimiAccount[]>('list_kimi_accounts');
export const startKimiLogin = (region: KimiRegion) =>
  invoke<KimiLogin>('start_kimi_login', { region });
export const pollKimiLogin = (loginId: string) =>
  invoke<KimiAccount | null>('poll_kimi_login', { loginId });
export const cancelKimiLogin = (loginId: string) => invoke<void>('cancel_kimi_login', { loginId });
export const refreshKimiAccount = (accountId: string) =>
  invoke<KimiAccount>('refresh_kimi_account', { accountId });
export const switchKimiAccount = (accountId: string) =>
  invoke<KimiAccount>('switch_kimi_account', { accountId });
export const deleteKimiAccount = (accountId: string) =>
  invoke<void>('delete_kimi_account', { accountId });
