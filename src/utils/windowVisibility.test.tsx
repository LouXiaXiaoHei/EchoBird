import { act, create } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useDocumentVisible } from '../hooks/useDocumentVisible';
import {
  getNativeWindowFocused,
  getNativeWindowVisible,
  initializeWindowVisibility,
} from './windowVisibility';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(), isTauri: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: mocks.isTauri }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

let event: (value: { payload: { visible: boolean; focused: boolean } }) => void;
let stop: (() => void) | undefined;
beforeEach(() => {
  vi.clearAllMocks();
  mocks.isTauri.mockReturnValue(true);
  mocks.invoke.mockResolvedValue({ visible: true, focused: true });
  mocks.listen.mockImplementation((_name, callback) => {
    event = callback;
    return Promise.resolve(vi.fn());
  });
});
afterEach(() => {
  stop?.();
  stop = undefined;
  vi.unstubAllGlobals();
});
const flush = async () => {
  for (let index = 0; index < 4; index++) await Promise.resolve();
};

it('starts paused and ignores a stale snapshot after a native event', async () => {
  let resolve!: (value: { visible: boolean; focused: boolean }) => void;
  mocks.invoke.mockReturnValue(new Promise((done) => (resolve = done)));
  stop = initializeWindowVisibility();
  expect(getNativeWindowVisible()).toBe(false);
  await flush();
  event({ payload: { visible: false, focused: false } });
  resolve({ visible: true, focused: true });
  await flush();
  expect(getNativeWindowVisible()).toBe(false);
  event({ payload: { visible: true, focused: true } });
  expect(getNativeWindowVisible()).toBe(true);
});

it('cleans up late subscriptions and never queries state after disposal', async () => {
  let resolve!: (cleanup: () => void) => void;
  const cleanup = vi.fn();
  mocks.listen.mockReturnValue(new Promise((done) => (resolve = done)));
  stop = initializeWindowVisibility();
  stop();
  resolve(cleanup);
  await flush();
  expect(cleanup).toHaveBeenCalledOnce();
  expect(mocks.invoke).not.toHaveBeenCalled();
});

it('shares one native subscription and hides both UI consumers while document stays visible', async () => {
  const doc = Object.assign(new EventTarget(), { visibilityState: 'visible' });
  vi.stubGlobal('document', doc);
  stop = initializeWindowVisibility();
  await flush();
  const values: boolean[] = [];
  function Consumer() {
    values.push(useDocumentVisible());
    return null;
  }
  let view!: ReturnType<typeof create>;
  act(() => {
    view = create(
      <>
        <Consumer />
        <Consumer />
      </>
    );
  });
  expect(values.slice(-2)).toEqual([true, true]);
  act(() => event({ payload: { visible: true, focused: false } }));
  expect(getNativeWindowFocused()).toBe(false);
  expect(values.slice(-2)).toEqual([true, true]);
  act(() => event({ payload: { visible: false, focused: false } }));
  expect(values.slice(-2)).toEqual([false, false]);
  act(() => event({ payload: { visible: true, focused: true } }));
  expect(values.slice(-2)).toEqual([true, true]);
  expect(mocks.listen).toHaveBeenCalledOnce();
  expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith('get_main_window_visibility');
  act(() => view.unmount());
});

it('keeps document visibility available in browser previews without native calls', () => {
  mocks.isTauri.mockReturnValue(false);
  stop = initializeWindowVisibility();
  expect(mocks.listen).not.toHaveBeenCalled();
  expect(mocks.invoke).not.toHaveBeenCalled();
});
