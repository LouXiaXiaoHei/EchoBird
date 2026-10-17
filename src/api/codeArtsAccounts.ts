import { invoke } from '@tauri-apps/api/core';

export interface CodeArtsAccount {
  id: string;
  email: string;
  toolId: 'codearts';
  plan?: string | null;
  balance?: number | null;
  total?: number | null;
  subscriptionEndAt?: number | null;
  dailyClaimedAt?: number | null;
  active: boolean;
}

export interface CodeArtsLogin {
  loginId: string;
  verificationUri: string;
  expiresAt: number;
}

export const listCodeArtsAccounts = () => invoke<CodeArtsAccount[]>('list_codearts_accounts');
export const startCodeArtsLogin = () => invoke<CodeArtsLogin>('start_codearts_login');
export const pollCodeArtsLogin = (loginId: string) =>
  invoke<CodeArtsAccount | null>('poll_codearts_login', { loginId });
export const cancelCodeArtsLogin = (loginId: string) =>
  invoke<void>('cancel_codearts_login', { loginId });
export const refreshCodeArtsAccount = (accountId: string) =>
  invoke<CodeArtsAccount>('refresh_codearts_account', { accountId });
export const switchCodeArtsAccount = (accountId: string) =>
  invoke<CodeArtsAccount>('switch_codearts_account', { accountId });
export const deleteCodeArtsAccount = (accountId: string) =>
  invoke<void>('delete_codearts_account', { accountId });
export const claimCodeArtsCredits = (accountId: string) =>
  invoke<CodeArtsAccount>('claim_codearts_credits', { accountId });
