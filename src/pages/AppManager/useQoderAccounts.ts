import * as api from '../../api/tauri';
import { useManagedAccounts } from './useManagedAccounts';
import { qoderRewardStatus } from './qoderAccountDisplay';

export function useQoderAccounts(
  edition: api.QoderEdition,
  enabled: boolean,
  showError: (error: string) => void,
  navigationKey: string
) {
  const managed = useManagedAccounts<api.QoderAccount, api.QoderLogin>(
    edition,
    enabled,
    false,
    () => {},
    showError,
    {
      list: () => api.listQoderAccounts(edition),
      start: () => api.startQoderLogin(edition),
      poll: (id) => api.pollQoderLogin(id),
      cancel: (id) => api.cancelQoderLogin(id),
      open: (login) => api.openExternal(login.verificationUri),
      remove: (account) => api.deleteQoderAccount(edition, account.id),
      refresh: (account) => api.refreshQoderAccountQuota(edition, account.id),
      label: (account) => account.name || account.email || account.id,
    },
    navigationKey
  );
  const claimDaily = (account: api.QoderAccount) => {
    const current = managed.accounts.find((row) => row.id === account.id);
    if (account.edition !== edition || !current) return Promise.resolve();
    const status = qoderRewardStatus(current);
    if (status === 'claimed' || status === 'unavailable') return Promise.resolve();
    return managed.refresh(current, (row) => api.claimQoderDailyCredits(edition, row.id));
  };
  return { ...managed, claimDaily };
}
