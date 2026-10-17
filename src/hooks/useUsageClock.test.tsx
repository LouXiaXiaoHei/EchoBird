import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, expect, it, vi } from 'vitest';
import { useUsageClock } from './useUsageClock';
const native = vi.hoisted(() => ({ visible: true, listeners: new Set<() => void>() }));
vi.mock('../utils/windowVisibility', () => ({
  getNativeWindowVisible: () => native.visible,
  subscribeWindowVisibility: (listener: () => void) => {
    native.listeners.add(listener);
    return () => native.listeners.delete(listener);
  },
}));
let view: ReactTestRenderer;
afterEach(() => {
  act(() => view?.unmount());
  vi.useRealTimers();
  vi.unstubAllGlobals();
  native.visible = true;
  native.listeners.clear();
});
it('shares one clock, pauses when hidden, and immediately corrects time on resume', () => {
  vi.useFakeTimers();
  vi.setSystemTime(100_000);
  const document = Object.assign(new EventTarget(), { visibilityState: 'visible' });
  vi.stubGlobal('document', document);
  function Clock({ active }: { active: boolean }) {
    return <span>{useUsageClock(active)}</span>;
  }
  const clocks = (active = true) => (
    <>
      {Array.from({ length: 30 }, (_, key) => (
        <Clock key={key} active={active} />
      ))}
    </>
  );
  act(() => {
    view = create(clocks());
  });
  expect(vi.getTimerCount()).toBe(1);
  act(() => {
    vi.advanceTimersByTime(60_000);
  });
  expect(view.root.findAllByType('span').every((row) => row.children[0] === '160000')).toBe(true);
  act(() => {
    document.visibilityState = 'hidden';
    document.dispatchEvent(new Event('visibilitychange'));
  });
  expect(vi.getTimerCount()).toBe(0);
  act(() => {
    vi.advanceTimersByTime(180_000);
    document.visibilityState = 'visible';
    document.dispatchEvent(new Event('visibilitychange'));
  });
  expect(view.root.findAllByType('span')[0].children).toEqual(['340000']);
  act(() => {
    native.visible = false;
    native.listeners.forEach((listener) => listener());
  });
  expect(vi.getTimerCount()).toBe(0);
  act(() => {
    native.visible = true;
    native.listeners.forEach((listener) => listener());
  });
  expect(vi.getTimerCount()).toBe(1);
  act(() => view.update(clocks(false)));
  expect(vi.getTimerCount()).toBe(0);
});
