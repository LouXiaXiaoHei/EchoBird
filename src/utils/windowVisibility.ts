import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

interface WindowVisibility {
  visible: boolean;
  focused: boolean;
}

let visible = true;
let focused: boolean | undefined;
const subscribers = new Set<() => void>();

export const getNativeWindowVisible = () => visible;
export const getNativeWindowFocused = () => focused;
export function subscribeWindowVisibility(callback: () => void) {
  subscribers.add(callback);
  return () => subscribers.delete(callback);
}

function update(value: WindowVisibility) {
  if (visible === value.visible && focused === value.focused) return;
  visible = value.visible;
  focused = value.focused;
  subscribers.forEach((callback) => callback());
}

// One native subscription for all animation and UI polling consumers.
export function initializeWindowVisibility(): () => void {
  if (!isTauri()) return () => {};
  update({ visible: false, focused: false });
  let disposed = false;
  let revision = 0;
  let unlisten: (() => void) | undefined;
  void listen<WindowVisibility>('main-window-visibility', (event) => {
    if (disposed) return;
    revision++;
    update(event.payload);
  })
    .then(async (cleanup) => {
      if (disposed) return cleanup();
      unlisten = cleanup;
      const initialRevision = revision;
      const initial = await invoke<WindowVisibility>('get_main_window_visibility');
      if (!disposed && revision === initialRevision) update(initial);
    })
    .catch((error) => {
      if (!disposed) console.warn('[WindowVisibility] Native state unavailable:', error);
    });
  return () => {
    disposed = true;
    unlisten?.();
  };
}
