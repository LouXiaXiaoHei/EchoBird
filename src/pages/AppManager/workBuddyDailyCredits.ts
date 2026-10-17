import { useUsageClock } from '../../hooks/useUsageClock';

export const workBuddyClaimedToday = (at?: number | null, now = Date.now()) =>
  at != null && Math.floor((at + 8 * 3600) / 86400) === Math.floor((now / 1000 + 8 * 3600) / 86400);

export function useWorkBuddyClaimedToday(enabled: boolean) {
  const now = useUsageClock(enabled);
  return (at?: number | null) => workBuddyClaimedToday(at, now);
}
