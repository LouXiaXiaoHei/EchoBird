import { useCallback, useState, useSyncExternalStore } from 'react';
import { getNativeWindowVisible, subscribeWindowVisibility } from '../utils/windowVisibility';

let now = Date.now();
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | undefined;
let unsubscribe: (() => void) | undefined;
const visible = () =>
  getNativeWindowVisible() &&
  (typeof document === 'undefined' || document.visibilityState === 'visible');
const tick = () => {
  now = Date.now();
  listeners.forEach((listener) => listener());
};
const synchronize = () => {
  if (timer !== undefined) clearInterval(timer);
  timer = undefined;
  if (listeners.size && visible()) {
    tick();
    timer = setInterval(tick, 60_000);
  }
};
function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    unsubscribe = subscribeWindowVisibility(synchronize);
    if (typeof document !== 'undefined') document.addEventListener('visibilitychange', synchronize);
    tick();
    synchronize();
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size) return;
    if (timer !== undefined) clearInterval(timer);
    timer = undefined;
    unsubscribe?.();
    if (typeof document !== 'undefined')
      document.removeEventListener('visibilitychange', synchronize);
  };
}
const snapshot = () => now;
// Local display updates only. Quota refreshes remain explicit.
export function useUsageClock(active: boolean): number {
  const observe = useCallback(
    (listener: () => void) => (active ? subscribe(listener) : () => {}),
    [active]
  );
  const [initial] = useState(Date.now);
  const read = useCallback(() => (active ? snapshot() : initial), [active, initial]);
  const serverSnapshot = useCallback(() => initial, [initial]);
  return useSyncExternalStore(observe, read, serverSnapshot);
}
