// AI-career dashboard selection and explicit refresh signal.

import { create } from 'zustand';
import type { AiCareerFamily } from '../api/aiCareer';

interface AiCareerState {
  selectedFamily: AiCareerFamily | null;
  setSelectedFamily: (family: AiCareerFamily | null) => void;
  chartMetric: 'messages' | 'tokens';
  setChartMetric: (metric: 'messages' | 'tokens') => void;
  refreshKey: number;
  refreshing: boolean;
  refresh: () => void;
  setRefreshing: (value: boolean) => void;
}

export const useAiCareerStore = create<AiCareerState>((set) => ({
  selectedFamily: null,
  setSelectedFamily: (family) => set({ selectedFamily: family }),
  chartMetric: 'tokens',
  setChartMetric: (metric) => set({ chartMetric: metric }),
  refreshKey: 0,
  refreshing: false,
  refresh: () => set((s) => ({ refreshKey: s.refreshKey + 1, refreshing: true })),
  setRefreshing: (value) => set({ refreshing: value }),
}));
