import { useEffect } from 'react';
import { claimAllSavedRewards } from '../services/accountRewardClaims';
import { useRewardClaimStore } from '../stores/rewardClaimStore';
import { useToolsStore } from '../stores/toolsStore';

export function useAutoClaimRewards() {
  const enabled = useRewardClaimStore((state) => state.enabled);
  const toolsReady = useToolsStore((state) => state.hasScanned);
  useEffect(() => {
    if (!enabled || !toolsReady) return;
    let active = true;
    let startupPending = true;
    const run = () => {
      if (!active || useToolsStore.getState().isScanning || useRewardClaimStore.getState().running)
        return;
      startupPending = false;
      void claimAllSavedRewards(() => active && useRewardClaimStore.getState().enabled);
    };
    const startup = setTimeout(run, 0);
    const hourly = setInterval(run, 60 * 60 * 1000);
    const stopTools = useToolsStore.subscribe(() => {
      if (startupPending) run();
    });
    const stopClaims = useRewardClaimStore.subscribe(() => {
      if (startupPending) run();
    });
    return () => {
      active = false;
      clearTimeout(startup);
      clearInterval(hourly);
      stopTools();
      stopClaims();
    };
  }, [enabled, toolsReady]);
}
