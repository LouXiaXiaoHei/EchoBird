// Theme store — light / dark / system, persisted to AppSettings.themeMode.
// Missing settings file defaults to dark; an unset saved themeMode means follow system.
import { create } from 'zustand';
import * as api from '../api/tauri';
import {
  getColorTheme,
  isColorThemeId,
  type ColorThemeId,
  type ColorThemeVariant,
} from '../data/colorThemes';

export type ThemeMode = 'system' | 'light' | 'dark';
type Resolved = 'light' | 'dark';

const mql = window.matchMedia('(prefers-color-scheme: dark)');

const resolve = (mode: ThemeMode): Resolved => {
  if (mode === 'system') return mql.matches ? 'dark' : 'light';
  return mode;
};

const hexToRgbChannels = (hex: string): string => {
  const value = Number.parseInt(hex.slice(1), 16);
  return `${(value >> 16) & 255} ${(value >> 8) & 255} ${value & 255}`;
};

const applyColorTheme = (id: ColorThemeId, resolved: Resolved) => {
  const theme = getColorTheme(id);
  const variant: ColorThemeVariant = resolved === 'dark' ? theme.dark : theme.light;
  const root = document.documentElement;
  root.style.setProperty('--bg-base-rgb', hexToRgbChannels(variant.canvas));
  root.style.setProperty('--bg-terminal-rgb', hexToRgbChannels(variant.shell));
  root.style.setProperty('--bg-surface-rgb', hexToRgbChannels(variant.secondary));
  root.style.setProperty('--bg-elevated-rgb', hexToRgbChannels(variant.tertiary));
  root.style.setProperty('--bg-deep-rgb', hexToRgbChannels(variant.shell));
  root.style.setProperty('--bg-input-rgb', hexToRgbChannels(variant.input));
  const tabColor = hexToRgbChannels(variant.tertiary);
  const channels = tabColor.split(' ').map(Number);
  // Match the character background's neutral-palette fallback without changing primary buttons.
  const neutral = Math.max(...channels) - Math.min(...channels) < 18;
  root.style.setProperty('--view-mode-accent-rgb', neutral ? 'var(--accent-rgb)' : tabColor);
  root.style.setProperty(
    '--view-mode-wave-rgb',
    neutral && resolved === 'light' ? '255 246 235' : 'var(--view-mode-accent-rgb)'
  );
  try {
    localStorage.setItem('color-theme', id);
  } catch {
    /* private mode */
  }
};

// Apply the resolved theme to <html data-theme> and persist a sync-readable
// copy to localStorage so index.html's inline pre-script can pick it up on
// next launch (avoids a wrong-theme flash before getSettings() resolves).
//
// `instant` swaps the theme without animating any color/background/border
// transitions — every element changes in the same frame, so theme switching
// looks like a clean swap rather than a janky stagger of `transition-colors`
// elements interpolating at different speeds.
const apply = (resolved: Resolved, colorTheme: ColorThemeId, instant: boolean) => {
  if (instant) {
    const kill = document.createElement('style');
    // Preserve the shared tab background's movement while theme colors swap instantly.
    kill.textContent =
      '*, *:not(.view-mode-tabs[data-sliding="true"])::before, *::after { transition: none !important; animation: none !important; }';
    document.head.appendChild(kill);
    document.documentElement.dataset.theme = resolved;
    applyColorTheme(colorTheme, resolved);
    // Force a reflow so the no-transition rule is in effect for this frame.
    void document.body?.offsetHeight;
    // Re-enable transitions on the next frame so future hover/state
    // changes still animate normally.
    requestAnimationFrame(() => requestAnimationFrame(() => kill.remove()));
  } else {
    document.documentElement.dataset.theme = resolved;
    applyColorTheme(colorTheme, resolved);
  }
  try {
    localStorage.setItem('theme', resolved);
  } catch {
    /* private mode */
  }
};

interface ThemeStore {
  mode: ThemeMode;
  resolved: Resolved;
  colorTheme: ColorThemeId;
  setMode: (mode: ThemeMode) => void;
  setColorTheme: (colorTheme: ColorThemeId) => void;
  init: () => Promise<void>;
}

export const useThemeStore = create<ThemeStore>((set, get) => ({
  mode: 'dark',
  resolved: 'dark',
  colorTheme: 'oatgray',
  setMode: (mode) => {
    const resolved = resolve(mode);
    apply(resolved, get().colorTheme, true);
    set({ mode, resolved });
    api
      .getSettings()
      .then((s) => {
        const current = get();
        return api.saveSettings({
          ...s,
          themeMode: current.mode === 'system' ? undefined : current.mode,
          colorTheme: current.colorTheme,
        });
      })
      .catch(() => {});
  },
  setColorTheme: (colorTheme) => {
    apply(get().resolved, colorTheme, true);
    set({ colorTheme });
    api
      .getSettings()
      .then((s) => {
        const current = get();
        return api.saveSettings({
          ...s,
          themeMode: current.mode === 'system' ? undefined : current.mode,
          colorTheme: current.colorTheme,
        });
      })
      .catch(() => {});
  },
  init: async () => {
    let mode: ThemeMode = 'system';
    let colorTheme: ColorThemeId = 'oatgray';
    // Restore the cached palette before the first await. index.html already
    // resolves the light/dark half synchronously, so this prevents the first
    // painted frame from briefly using EchoBird's default background colors
    // while the Rust settings file is still loading.
    try {
      const cachedColorTheme = localStorage.getItem('color-theme');
      if (isColorThemeId(cachedColorTheme)) colorTheme = cachedColorTheme;
    } catch {
      /* private mode */
    }
    const cachedResolved: Resolved =
      document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
    applyColorTheme(colorTheme, cachedResolved);
    try {
      const s = await api.getSettings();
      if (s.themeMode === 'light' || s.themeMode === 'dark') mode = s.themeMode;
      if (isColorThemeId(s.colorTheme)) colorTheme = s.colorTheme;
    } catch {
      /* default to system */
    }
    const resolved = resolve(mode);
    // No `instant: true` — initial paint has nothing to transition from.
    apply(resolved, colorTheme, false);
    set({ mode, resolved, colorTheme });
    mql.addEventListener('change', () => {
      if (get().mode !== 'system') return;
      const r: Resolved = mql.matches ? 'dark' : 'light';
      apply(r, get().colorTheme, true);
      set({ resolved: r });
    });
  },
}));
