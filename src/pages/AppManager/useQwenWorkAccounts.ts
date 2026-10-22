import * as api from '../../api/tauri';
import { useManagedAccounts } from './useManagedAccounts';
import { qwenWorkIdentity } from './qwenWorkAccountDisplay';

export function useQwenWorkAccounts(
  edition: api.QwenWorkEdition,
  enabled: boolean,
  clearModel: () => void,
  showError: (error: string) => void,
  navigationKey: string
) {
  return useManagedAccounts<api.QwenWorkAccount, api.QwenWorkLogin>(
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
}
