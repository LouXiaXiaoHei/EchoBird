import { useEffect, useRef, useState } from 'react';
import { useAppManager } from '../pages/AppManager/context';
import { useDocumentVisible } from '../hooks/useDocumentVisible';
import { createCharacterRenderer, type CharacterRenderer } from './desktopCharacterRenderer';

const motionPreferences = [
  '(prefers-reduced-motion: reduce)',
  '(prefers-contrast: more)',
  '(forced-colors: active)',
];

export function DesktopCharacterBackground() {
  const { selectedTool } = useAppManager();
  const visible = useDocumentVisible();
  const [reduced, setReduced] = useState(() =>
    motionPreferences.some((query) => window.matchMedia(query).matches)
  );
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const hostRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<CharacterRenderer | null>(null);
  const active = visible && !reduced;

  useEffect(() => {
    const preferences = motionPreferences.map((query) => window.matchMedia(query));
    const update = () => setReduced(preferences.some((preference) => preference.matches));
    preferences.forEach((preference) => preference.addEventListener('change', update));
    return () => {
      preferences.forEach((preference) => preference.removeEventListener('change', update));
    };
  }, []);

  useEffect(() => {
    const host = hostRef.current;
    if (!active || !host) return;
    // Each effect owns its canvas. StrictMode and hot reload can run cleanup
    // followed by setup on the same host; a lost context cannot be reused.
    const canvas = document.createElement('canvas');
    canvas.className = 'block h-full w-full';
    canvas.dataset.desktopCharacterBackground = '';
    host.appendChild(canvas);
    canvasRef.current = canvas;
    const renderer = createCharacterRenderer(canvas);
    rendererRef.current = renderer;
    if (!renderer) {
      canvas.remove();
      canvasRef.current = null;
    }
    return () => {
      // Detach before releasing WebGL so its native failure placeholder cannot
      // cover the interface while the next effect is being created.
      canvas.remove();
      renderer?.dispose();
      canvasRef.current = null;
      rendererRef.current = null;
    };
  }, [active]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!active || !canvas || !selectedTool) return;
    const bounds = canvas.getBoundingClientRect();
    const icon = Array.from(document.querySelectorAll<HTMLElement>('[data-desktop-tool-id]')).find(
      (element) => element.dataset.desktopToolId === selectedTool && element.getClientRects().length
    );
    const rect = icon?.getBoundingClientRect();
    rendererRef.current?.select(selectedTool, {
      x: rect && bounds.width ? (rect.left + rect.width / 2 - bounds.left) / bounds.width : 0.5,
      y: rect && bounds.height ? (rect.top + rect.height / 2 - bounds.top) / bounds.height : 0.5,
    });
  }, [active, selectedTool]);

  return active ? (
    <div
      ref={hostRef}
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 -z-10 h-full w-full"
      data-character-background-host=""
    />
  ) : null;
}
