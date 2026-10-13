import { invoke } from '@tauri-apps/api/core';
import type { AccountProvider, AccountSource } from './types';

export const accountProxyProviders: { id: AccountProvider; name: string; icon: string }[] = [
  { id: 'claudeCode', name: 'Claude Code', icon: 'claudecode.svg' },
  { id: 'codex', name: 'ChatGPT / Codex', icon: 'chatgptdesktop.svg' },
  { id: 'workBuddy', name: 'WorkBuddy', icon: 'workbuddy.png' },
  { id: 'workBuddyAi', name: 'WorkBuddy AI', icon: 'workbuddyai.png' },
  { id: 'antigravity', name: 'Antigravity', icon: 'antigravity.png' },
  { id: 'trae', name: 'Trae', icon: 'trae.svg' },
  { id: 'traecn', name: 'Trae CN', icon: 'traecn.svg' },
  { id: 'kilo', name: 'Kilo', icon: 'kilodesktop.png' },
  { id: 'copilot', name: 'GitHub Copilot', icon: 'copilotdesktop.png' },
];

export interface ProxyAccount {
  source: AccountSource;
  label: string;
  plan?: string;
}

export function listProxyAccounts(provider: AccountProvider): Promise<ProxyAccount[]> {
  return invoke('list_proxy_accounts', { provider });
}

export function getAccountProxyModels(source: AccountSource): Promise<string[]> {
  return invoke('get_account_proxy_models', { source });
}

function modelCacheKey(source: AccountSource) {
  return `echobird-account-models:${JSON.stringify([source.provider, source.accountId])}`;
}

export function getCachedAccountProxyModels(source: AccountSource): string[] | undefined {
  try {
    const models = JSON.parse(localStorage.getItem(modelCacheKey(source)) ?? 'null');
    if (Array.isArray(models) && models.every((id) => typeof id === 'string')) return models;
  } catch {
    // An unavailable or damaged cache falls back to reading the account directory.
  }
  return undefined;
}

export function cacheAccountProxyModels(source: AccountSource, models: string[]): void {
  try {
    localStorage.setItem(modelCacheKey(source), JSON.stringify(models));
  } catch {
    // Keep the current directory usable if persistence is unavailable.
  }
}
