import * as api from '../../api/tauri';
import { BROWSER_LOGIN_TIMEOUT_SECONDS, useManagedAccounts } from './useManagedAccounts';

const clients = {
  grok: {
    list: api.listGrokAccounts,
    start: api.startGrokLogin,
    cancel: api.cancelGrokLogin,
    remove: api.deleteGrokAccount,
    switch: api.switchGrokAccount,
    refresh: api.refreshGrokAccount,
  },
  manus: {
    list: api.listManusAccounts,
    start: api.startManusLogin,
    cancel: api.cancelManusLogin,
    remove: api.deleteManusAccount,
    switch: api.switchManusAccount,
    refresh: api.refreshManusAccount,
  },
  cue: {
    list: api.listCueAccounts,
    start: api.startCueLogin,
    cancel: api.cancelCueLogin,
    remove: api.deleteCueAccount,
    switch: api.switchCueAccount,
    refresh: api.refreshCueAccount,
  },
};

export function useGrokAccounts(
  enabled: boolean,
  hasModel: boolean,
  clearModel: () => void,
  showError: (error: string) => void,
  tool: 'grok' | 'manus' | 'cue' = 'grok'
) {
  const client = clients[tool];
  const managed = useManagedAccounts<
    api.GrokAccount | api.ManusAccount | api.CueAccount,
    api.GrokLogin | api.ManusLogin
  >(tool, enabled, hasModel, clearModel, showError, {
    ...client,
    start: client.start,
    poll: async (id) => {
      if (tool === 'grok') return api.pollGrokLogin(id);
      return (await (tool === 'cue' ? api.pollCueLogin(id) : api.pollManusLogin(id))).account;
    },
    cancel: client.cancel,
    remove: (account) => client.remove(account.id),
    refresh: (account) => client.refresh(account.id),
    label: (account) => account.email,
    loginTimeoutSeconds: tool === 'grok' ? 60 : BROWSER_LOGIN_TIMEOUT_SECONDS,
  });
  const switchAccount = async () => {
    if (!managed.selectedId) return;
    await client.switch(managed.selectedId);
    await managed.reload();
  };
  return {
    ...managed,
    awaitingClientExit: false,
    switchAccount,
  };
}
