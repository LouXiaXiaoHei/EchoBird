import { create } from 'zustand';

const STORAGE_KEY = 'echobird_auto_claim_rewards';

function readEnabled(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
}

interface RewardClaimState {
  enabled: boolean;
  running: boolean;
  updatedScope: string | null;
  revision: number;
  setEnabled: (enabled: boolean) => void;
  tryStart: () => boolean;
  finish: () => void;
  updated: (scope: string) => void;
}

export const useRewardClaimStore = create<RewardClaimState>((set, get) => ({
  enabled: readEnabled(),
  running: false,
  updatedScope: null,
  revision: 0,
  setEnabled: (enabled) => {
    set({ enabled });
    try {
      localStorage.setItem(STORAGE_KEY, String(enabled));
    } catch {
      // Keep the current session's choice when storage is unavailable.
    }
  },
  tryStart: () => {
    if (get().running) return false;
    set({ running: true });
    return true;
  },
  finish: () => set({ running: false }),
  updated: (scope) => set((state) => ({ updatedScope: scope, revision: state.revision + 1 })),
}));
