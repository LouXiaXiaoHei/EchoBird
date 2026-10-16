import { useRef } from 'react';
import * as api from '../../api/tauri';
import { useManagedAccounts } from './useManagedAccounts';

export function useOpenDesignAccounts(
  enabled: boolean,
  hasModel: boolean,
  clearModel: () => void,
  showError: (error: string) => void
) {
  const pollInterval = useRef(2000);
  return useManagedAccounts<api.OpenDesignAccount, api.OpenDesignLogin>(
    'opendesign',
    enabled,
    hasModel,
    clearModel,
    showError,
    {
      list: api.listOpenDesignAccounts,
      start: api.startOpenDesignLogin,
      open: (login) => {
        pollInterval.current = login.pollIntervalSeconds * 1000;
        return api.openExternal(login.verificationUri);
      },
      poll: async (id) => {
        const account = await api.pollOpenDesignLogin(id);
        if (!account) await new Promise((resolve) => setTimeout(resolve, pollInterval.current));
        return account;
      },
      pollInterval: 0,
      cancel: api.cancelOpenDesignLogin,
      remove: (account) => api.deleteOpenDesignAccount(account.id),
      refresh: (account) => api.refreshOpenDesignAccount(account.id),
      label: (account) => account.email,
    }
  );
}
