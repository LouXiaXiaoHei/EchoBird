import type { DoubaoQuotaWindow } from '../../api/doubao';

export function doubaoRemainingPercent(window: {
  remaining: number | null;
  total: number | null;
  usedPercent: number | null;
  unlimited?: boolean;
}): number | undefined {
  const percent = window.unlimited
    ? 100
    : window.usedPercent != null
      ? 100 - window.usedPercent
      : window.remaining != null && window.total != null && window.total > 0
        ? (window.remaining / window.total) * 100
        : undefined;
  return percent != null && Number.isFinite(percent)
    ? Math.max(0, Math.min(100, percent))
    : undefined;
}

export function doubaoWindowLabel(
  window: DoubaoQuotaWindow,
  windows: DoubaoQuotaWindow[],
  subscriptionLabel: string
): string {
  const label = window.kind === 'subscription' ? subscriptionLabel : window.kind;
  return window.label && windows.filter((row) => row.kind === window.kind).length > 1
    ? `${window.label} ${label}`
    : label;
}
