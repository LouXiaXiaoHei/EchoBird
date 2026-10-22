import * as api from '../../api/tauri';
import { BROWSER_LOGIN_TIMEOUT_SECONDS, useManagedAccounts } from './useManagedAccounts';
import { dumateRewardStatus } from './dumateAccountDisplay';

export function useDuMateAccounts(
  enabled: boolean,
  clearModel: () => void,
  showError: (error: string) => void,
  navigationKey: string
) {
  const managed = useManagedAccounts<api.DuMateAccount, api.DuMateLogin>(
    'dumate',
    enabled,
    false,
    clearModel,
    showError,
    {
      list: () => api.listDuMateAccounts(),
      start: () => api.startDuMateLogin(),
      poll: (id) => api.pollDuMateLogin(id),
      cancel: (id) => api.cancelDuMateLogin(id),
      remove: (a) => api.deleteDuMateAccount(a.id),
      refresh: (a) => api.refreshDuMateAccountQuota(a.id),
      label: (a) => a.name || a.id,
      loginTimeoutSeconds: BROWSER_LOGIN_TIMEOUT_SECONDS,
    },
    navigationKey
  );
  const claimDaily = (account: api.DuMateAccount, onError?: (error: unknown) => void) => {
    const current = managed.accounts.find((row) => row.id === account.id);
    if (!current || ['claimed', 'unavailable'].includes(dumateRewardStatus(current)))
      return Promise.resolve();
    return managed.refresh(current, (row) => api.claimDuMateDailyCredits(row.id), onError);
  };
  return { ...managed, claimDaily };
}
