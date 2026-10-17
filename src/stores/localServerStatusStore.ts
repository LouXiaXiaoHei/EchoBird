import { create } from 'zustand';

export const useLocalServerStatusStore = create<{
  running: boolean;
  revision: number;
  setRunning: (running: boolean) => void;
}>((set) => ({
  running: false,
  revision: 0,
  setRunning: (running) => set((state) => ({ running, revision: state.revision + 1 })),
}));
