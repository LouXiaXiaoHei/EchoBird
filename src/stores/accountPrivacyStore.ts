import { create } from 'zustand';

export const ACCOUNT_NAME_MASK = '******';
// Keep the original Smart Access preference across all account views.
const STORAGE_KEY = 'echobird-router-account-names-hidden';

function readHidden(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
}

interface AccountPrivacyState {
  hidden: boolean;
  setHidden: (hidden: boolean) => void;
}

export const useAccountPrivacyStore = create<AccountPrivacyState>((set) => ({
  hidden: readHidden(),
  setHidden: (hidden) => {
    set({ hidden });
    try {
      localStorage.setItem(STORAGE_KEY, String(hidden));
    } catch {
      // Keep the current choice if local storage is unavailable.
    }
  },
}));
