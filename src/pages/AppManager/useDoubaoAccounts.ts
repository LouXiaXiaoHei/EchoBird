import * as api from '../../api/tauri';
import { BROWSER_LOGIN_TIMEOUT_SECONDS, useManagedAccounts } from './useManagedAccounts';

export function useDoubaoAccounts(
  edition: api.DoubaoEdition,
  enabled: boolean,
  hasModel: boolean,
  clearModel: () => void,
  showError: (error: string) => void,
  navigationKey: string
) {
  const managed = useManagedAccounts<api.DoubaoAccount, api.DoubaoLogin>(
    edition,
    enabled,
    hasModel,
    clearModel,
    showError,
    {
      list: () => api.listDoubaoAccounts(edition),
      start: () => api.startDoubaoLogin(edition),
      poll: (id) => api.pollDoubaoLogin(id),
      cancel: (id) => api.cancelDoubaoLogin(id),
      remove: (account) => api.deleteDoubaoAccount(edition, account.id),
      refresh: (account) => api.refreshDoubaoAccountQuota(edition, account.id),
      label: (account) => account.name || account.email || account.id,
      loginTimeoutSeconds: BROWSER_LOGIN_TIMEOUT_SECONDS,
    },
    navigationKey
  );
  const claimDaily = (account: api.DoubaoAccount, onError?: (error: unknown) => void) => {
    const current = managed.accounts.find((row) => row.id === account.id);
    if (account.edition !== edition || !current || current.dailyReward?.status === 'unavailable')
      return Promise.resolve();
    return managed.refresh(current, (row) => api.claimDoubaoDailyCredits(edition, row.id), onError);
  };
  return { ...managed, claimDaily };
}
