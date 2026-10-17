import { useEffect, useState, useSyncExternalStore } from 'react';
import { getNativeWindowVisible, subscribeWindowVisibility } from '../utils/windowVisibility';

/**
 * Combines document visibility with native hide/minimize state. WebView2
 * can report a visible document even when its parent window is hidden.
 * Focus alone does not determine whether UI polling should continue.
 */
export function useDocumentVisible(): boolean {
  const nativeVisible = useSyncExternalStore(
    subscribeWindowVisibility,
    getNativeWindowVisible,
    getNativeWindowVisible
  );
  const [visible, setVisible] = useState(
    () => typeof document !== 'undefined' && document.visibilityState === 'visible'
  );
  useEffect(() => {
    const onChange = () => setVisible(document.visibilityState === 'visible');
    document.addEventListener('visibilitychange', onChange);
    return () => document.removeEventListener('visibilitychange', onChange);
  }, []);
  return visible && nativeVisible;
}
