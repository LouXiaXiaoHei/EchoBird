import { describe, expect, it } from 'vitest';
import { COLOR_THEMES, getColorTheme } from '../data/colorThemes';
import { characterInk } from './desktopCharacterRenderer';

const rgb = (hex: string): [number, number, number] => [
  parseInt(hex.slice(1, 3), 16),
  parseInt(hex.slice(3, 5), 16),
  parseInt(hex.slice(5, 7), 16),
];
const accent: [number, number, number] = [217, 119, 87];

describe('character background theme color', () => {
  it('uses the actual orange UI accent for the default neutral palette', () => {
    const ink = characterInk(rgb(getColorTheme('oatgray').dark.tertiary), accent, true);
    expect(ink[0]).toBeGreaterThan(ink[1]);
    expect(ink[1]).toBeGreaterThan(ink[2]);
    const updated = characterInk([40, 40, 40], [80, 200, 100], true);
    expect(updated[1]).toBeGreaterThan(updated[0]);
    expect(updated[1]).toBeGreaterThan(updated[2]);
  });

  it.each(['light', 'dark'] as const)(
    'follows blue, green and purple palette hues in %s mode',
    (mode) => {
      const blue = characterInk(
        rgb(getColorTheme('cobalt')[mode].tertiary),
        accent,
        mode === 'dark'
      );
      const green = characterInk(
        rgb(getColorTheme('forest')[mode].tertiary),
        accent,
        mode === 'dark'
      );
      const purple = characterInk(
        rgb(getColorTheme('lavender')[mode].tertiary),
        accent,
        mode === 'dark'
      );
      expect(blue[2]).toBeGreaterThan(blue[0]);
      expect(blue[2]).toBeGreaterThan(blue[1]);
      expect(green[1]).toBeGreaterThan(green[0]);
      expect(green[1]).toBeGreaterThan(green[2]);
      expect(purple[0]).toBeGreaterThan(purple[1]);
      expect(purple[2]).toBeGreaterThan(purple[1]);
    }
  );

  it.each(COLOR_THEMES.map((theme) => [theme.id, theme] as const))(
    'keeps %s legible and more subtle in light mode',
    (_id, theme) => {
      const dark = characterInk(rgb(theme.dark.tertiary), accent, true);
      const light = characterInk(rgb(theme.light.tertiary), accent, false);
      for (const channel of [...dark, ...light]) {
        expect(Number.isFinite(channel)).toBe(true);
        expect(channel).toBeGreaterThanOrEqual(0);
        expect(channel).toBeLessThanOrEqual(1);
      }
      expect(light[3]).toBeLessThan(dark[3]);
      expect(Math.max(...light.slice(0, 3))).toBeLessThan(Math.max(...dark.slice(0, 3)));
    }
  );
});
