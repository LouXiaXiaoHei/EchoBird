import * as api from '../../api/devin';
import { openExternal } from '../../api/tauri';
import { useManagedAccounts } from './useManagedAccounts';

export function useDevinAccounts(
  tool: api.DevinTool,
  enabled: boolean,
  showError: (error: string) => void,
  navigationKey: string
) {
  return useManagedAccounts<api.DevinAccount, api.DevinLogin>(
    'devin',
    enabled,
    false,
    () => {},
    showError,
    {
      list: () => api.listDevinAccounts(tool),
      start: () => api.startDevinLogin(tool),
      poll: api.pollDevinLogin,
      cancel: api.cancelDevinLogin,
      open: (login) => openExternal(login.verificationUri),
      remove: (account) => api.deleteDevinAccount(account.id),
      refresh: (account) => api.refreshDevinAccount(account.id),
      label: (account) => account.email,
    },
    navigationKey
  );
}
