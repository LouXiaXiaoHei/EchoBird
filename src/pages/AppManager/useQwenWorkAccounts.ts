import * as api from '../../api/tauri';
import { useManagedAccounts } from './useManagedAccounts';
import { qwenWorkIdentity, qwenWorkRewardStatus } from './qwenWorkAccountDisplay';

export function useQwenWorkAccounts(
  edition: api.QwenWorkEdition,
  enabled: boolean,
  clearModel: () => void,
  showError: (error: string) => void,
  navigationKey: string
) {
  const managed = useManagedAccounts<api.QwenWorkAccount, api.QwenWorkLogin>(
    edition,
    enabled,
    false,
    clearModel,
    showError,
    {
      list: () => api.listQwenWorkAccounts(edition),
      start: () => api.startQwenWorkLogin(edition),
      poll: (id) => api.pollQwenWorkLogin(id),
      cancel: (id) => api.cancelQwenWorkLogin(id),
      open: (login) => api.openExternal(login.verificationUri),
      remove: (account) => api.deleteQwenWorkAccount(edition, account.id),
      refresh: (account) => api.refreshQwenWorkAccountQuota(edition, account.id),
      label: qwenWorkIdentity,
    },
    navigationKey
  );
  const claimDaily = (account: api.QwenWorkAccount, onError?: (error: unknown) => void) => {
    if (edition !== 'qwenworkcn') return Promise.resolve();
    const current = managed.accounts.find((row) => row.id === account.id);
    if (account.edition !== edition || !current) return Promise.resolve();
    const status = qwenWorkRewardStatus(current);
    if (status === 'claimed' || status === 'unavailable') return Promise.resolve();
    return managed.refresh(
      current,
      (row) => api.claimQwenWorkDailyCredits(edition, row.id),
      onError
    );
  };
  return { ...managed, claimDaily };
}
