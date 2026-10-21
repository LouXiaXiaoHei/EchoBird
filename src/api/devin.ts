import { invoke } from '@tauri-apps/api/core';
export type DevinTool = 'devin' | 'devincli';
export interface DevinAccount {
  id: string;
  email: string;
  active: boolean;
  plan?: string | null;
  quotaWindows: { label: string; remainingPercent: number; resetAt?: number | null }[];
  balanceUsd?: number | null;
  subscriptionEndAt?: number | null;
}
export interface DevinLogin {
  loginId: string;
  verificationUri: string;
  expiresAt: number;
}
export const listDevinAccounts = (toolId: DevinTool): Promise<DevinAccount[]> =>
  invoke('list_devin_accounts', { toolId });
export const startDevinLogin = (toolId: DevinTool): Promise<DevinLogin> =>
  invoke('start_devin_login', { toolId });
export const pollDevinLogin = (loginId: string): Promise<DevinAccount | null> =>
  invoke('poll_devin_login', { loginId });
export const cancelDevinLogin = (loginId: string): Promise<void> =>
  invoke('cancel_devin_login', { loginId });
export const switchDevinAccount = (accountId: string, toolId: DevinTool): Promise<DevinAccount> =>
  invoke('switch_devin_account', { accountId, toolId });
export const refreshDevinAccount = (accountId: string): Promise<DevinAccount> =>
  invoke('refresh_devin_account', { accountId });
export const deleteDevinAccount = (accountId: string): Promise<void> =>
  invoke('delete_devin_account', { accountId });
