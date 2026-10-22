import React, { useSyncExternalStore } from 'react';
import { useUsageClock } from '../../hooks/useUsageClock';

const subscribeMounted = () => () => {};
const clientMounted = () => true;
const serverMounted = () => false;

function formatQuotaCountdown(resetAt: number, now: number, compact?: boolean): string {
  const minutes = Math.max(0, Math.ceil((resetAt * 1000 - now) / 60_000));
  if (minutes >= (compact ? 48 : 24) * 60) {
    const days = Math.floor(minutes / (24 * 60));
    const hours = Math.floor((minutes % (24 * 60)) / 60);
    return `${days}d${hours}h`;
  }
  if (minutes >= 60) {
    return `${Math.floor(minutes / 60)}h${minutes % 60}m`;
  }
  return `${minutes}m`;
}

export const QuotaCountdown: React.FC<{
  resetAt?: number | null;
  compact?: boolean;
  small?: boolean;
  parenthesized?: boolean;
  label?: string;
}> = ({ resetAt, compact, small, parenthesized, label }) => {
  const now = useUsageClock(!!resetAt);
  const mounted = useSyncExternalStore(subscribeMounted, clientMounted, serverMounted);
  if (compact && (!mounted || !resetAt || resetAt * 1000 <= now)) return null;
  const countdown = resetAt && mounted ? formatQuotaCountdown(resetAt, now, compact) : '';
  return (
    <span
      aria-label={label && countdown ? `${label} ${countdown}` : undefined}
      className={`${compact ? '' : 'w-[64px] '}inline-flex flex-shrink-0 items-center justify-center align-middle text-center ${small ? 'text-[10px]' : 'text-[12px]'} font-semibold leading-[16px] text-cyber-text`}
    >
      {parenthesized && countdown ? `(${countdown})` : countdown}
    </span>
  );
};
