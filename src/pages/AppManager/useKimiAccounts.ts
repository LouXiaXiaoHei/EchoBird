import { useRef, useState } from 'react';
import * as api from '../../api/kimiAccounts';
import { openExternal } from '../../api/tauri';
import { useManagedAccounts } from './useManagedAccounts';

export function useKimiAccounts(
  enabled: boolean,
  hasModel: boolean,
  clearModel: () => void,
  showError: (error: string) => void,
  navigationKey: string
) {
  const [region, setRegion] = useState<api.KimiRegion>('cn');
  const interval = useRef(5000);
  const managed = useManagedAccounts<api.KimiAccount, api.KimiLogin>(
    navigationKey,
    enabled,
    hasModel,
    clearModel,
    showError,
    {
      list: api.listKimiAccounts,
      start: () => api.startKimiLogin(region),
      open: (login) => {
        interval.current = login.pollIntervalSeconds * 1000;
        return openExternal(login.verificationUri);
      },
      poll: async (id) => {
        const account = await api.pollKimiLogin(id);
        if (!account) await new Promise((resolve) => setTimeout(resolve, interval.current));
        return account;
      },
      pollInterval: 0,
      cancel: api.cancelKimiLogin,
      remove: (account) => api.deleteKimiAccount(account.id),
      refresh: (account) => api.refreshKimiAccount(account.id),
      label: (account) => account.email,
    },
    navigationKey
  );
  return {
    ...managed,
    region,
    setRegion: (value: api.KimiRegion) => {
      if (!managed.busy) setRegion(value);
    },
  };
}
