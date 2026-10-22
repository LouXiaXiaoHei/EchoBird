import * as api from '../../api/codeArtsAccounts';
import { openExternal } from '../../api/tauri';
import { useManagedAccounts } from './useManagedAccounts';

export function useCodeArtsAccounts(
  enabled: boolean,
  hasModel: boolean,
  clearModel: () => void,
  showError: (error: string) => void,
  navigationKey: string
) {
  const managed = useManagedAccounts<api.CodeArtsAccount, api.CodeArtsLogin>(
    'codearts',
    enabled,
    hasModel,
    clearModel,
    showError,
    {
      list: api.listCodeArtsAccounts,
      start: api.startCodeArtsLogin,
      open: (login) => openExternal(login.verificationUri),
      poll: api.pollCodeArtsLogin,
      cancel: api.cancelCodeArtsLogin,
      refresh: (account) => api.refreshCodeArtsAccount(account.id),
      remove: (account) => api.deleteCodeArtsAccount(account.id),
      label: (account) => account.email,
    },
    navigationKey
  );
  return {
    ...managed,
    claimDaily: (account: api.CodeArtsAccount, onError?: (error: unknown) => void) =>
      managed.refresh(account, (row) => api.claimCodeArtsCredits(row.id), onError),
  };
}
