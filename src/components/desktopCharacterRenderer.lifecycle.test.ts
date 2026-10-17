import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createCharacterRenderer, type CharacterRenderer } from './desktopCharacterRenderer';

const native = vi.hoisted(() => ({
  focused: undefined as boolean | undefined,
  notify: undefined as (() => void) | undefined,
  unsubscribe: vi.fn(),
}));
vi.mock('../utils/windowVisibility', () => ({
  getNativeWindowFocused: () => native.focused,
  subscribeWindowVisibility: (callback: () => void) => {
    native.notify = callback;
    return native.unsubscribe;
  },
}));

let renderer: CharacterRenderer | null;
let win: EventTarget;
let doc: EventTarget & { visibilityState: string };
let focused: boolean;
let offset: number;
let late: number;
let cost: number;
let gpuReady: boolean;
let software: boolean;
let mutation: () => void;
let resize: () => void;
let canvas: HTMLCanvasElement;
let gl: Record<string, unknown> &
  Record<
    | 'createShader'
    | 'createProgram'
    | 'createTexture'
    | 'drawArraysInstanced'
    | 'deleteProgram'
    | 'deleteTexture',
    ReturnType<typeof vi.fn>
  >;
let time: number;
let rect: { width: number; height: number };

beforeEach(() => {
  vi.useFakeTimers();
  native.focused = undefined;
  native.notify = undefined;
  native.unsubscribe.mockClear();
  focused = true;
  offset = late = cost = time = 0;
  gpuReady = true;
  software = false;
  rect = { width: 1000, height: 650 };
  const now = performance.now.bind(performance);
  vi.spyOn(performance, 'now').mockImplementation(() => now() + offset);
  gl = {
    VERTEX_SHADER: 1,
    FRAGMENT_SHADER: 2,
    COMPILE_STATUS: 3,
    LINK_STATUS: 4,
    SYNC_STATUS: 5,
    SIGNALED: 6,
    createShader: vi.fn(() => ({})),
    createProgram: vi.fn(() => ({})),
    createTexture: vi.fn(() => ({})),
    getShaderParameter: () => true,
    deleteProgram: vi.fn(),
    deleteTexture: vi.fn(),
    getProgramParameter: () => true,
    getUniformLocation: (_program: unknown, name: string) => name,
    uniform1f: (name: string, value: number) => {
      if (name === 'u_time') time = value;
    },
    getSyncParameter: () => (gpuReady ? 6 : 0),
    fenceSync: vi.fn(() => ({})),
    deleteSync: vi.fn(),
    isContextLost: () => false,
    getExtension: (name: string) =>
      name === 'WEBGL_debug_renderer_info'
        ? { UNMASKED_RENDERER_WEBGL: 7 }
        : { loseContext: vi.fn() },
    getParameter: () => (software ? 'SwiftShader' : 'hardware renderer'),
    drawArraysInstanced: vi.fn(() => {
      offset += cost;
    }),
  };
  for (const name of [
    'shaderSource',
    'compileShader',
    'attachShader',
    'linkProgram',
    'bindTexture',
    'texParameteri',
    'texImage2D',
    'useProgram',
    'uniform2f',
    'uniform4f',
    'viewport',
    'flush',
    'deleteShader',
    'deleteTexture',
    'deleteProgram',
  ])
    gl[name] = vi.fn();
  canvas = Object.assign(new EventTarget(), {
    style: {},
    width: 0,
    height: 0,
    getContext: vi.fn(() => gl),
    getBoundingClientRect: () => rect,
  }) as unknown as HTMLCanvasElement;
  doc = Object.assign(new EventTarget(), {
    visibilityState: 'visible',
    hasFocus: () => focused,
    documentElement: { dataset: { theme: 'dark' } },
    createElement: () => ({ getContext: () => ({ fillText: vi.fn() }) }),
  });
  win = Object.assign(new EventTarget(), {
    devicePixelRatio: 1,
    setTimeout: globalThis.setTimeout,
    clearTimeout: globalThis.clearTimeout,
  });
  vi.stubGlobal('window', win);
  vi.stubGlobal('document', doc);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) =>
    setTimeout(() => {
      const stamp = performance.now();
      offset += late;
      callback(stamp);
    }, 16)
  );
  vi.stubGlobal('cancelAnimationFrame', (id: number) => clearTimeout(id));
  vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: () => '100 120 140' }));
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(callback: () => void) {
        resize = callback;
      }
      observe() {}
      disconnect() {}
    }
  );
  vi.stubGlobal(
    'MutationObserver',
    class {
      constructor(callback: () => void) {
        mutation = callback;
      }
      observe() {}
      disconnect() {}
    }
  );
});

afterEach(() => {
  renderer?.dispose();
  renderer = null;
  vi.restoreAllMocks();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function start() {
  renderer = createCharacterRenderer(canvas);
  expect(renderer).not.toBeNull();
  vi.advanceTimersByTime(1000);
  expect(gl.drawArraysInstanced).toHaveBeenCalled();
}
const draws = () => gl.drawArraysInstanced.mock.calls.length;

it('stops on blur and resumes the same resources without jumping animation time', () => {
  start();
  const count = draws(),
    before = time;
  focused = false;
  win.dispatchEvent(new Event('blur'));
  vi.advanceTimersByTime(600_000);
  expect(draws()).toBe(count);
  expect(vi.getTimerCount()).toBe(0);
  focused = true;
  win.dispatchEvent(new Event('focus'));
  vi.advanceTimersByTime(1000);
  expect(draws()).toBeGreaterThan(count);
  expect(time - before).toBeLessThan(2);
  expect(gl.createProgram).toHaveBeenCalledOnce();
  expect(gl.createTexture).toHaveBeenCalledOnce();
});

it('uses native focus when the document focus stays stale and resumes the same renderer', () => {
  start();
  const count = draws();
  native.focused = false;
  native.notify!();
  vi.advanceTimersByTime(600_000);
  expect(draws()).toBe(count);
  expect(vi.getTimerCount()).toBe(0);
  expect(gl.deleteProgram).not.toHaveBeenCalled();
  focused = false;
  native.focused = true;
  native.notify!();
  vi.advanceTimersByTime(1000);
  expect(draws()).toBeGreaterThan(count);
  expect(gl.createProgram).toHaveBeenCalledOnce();
  expect(gl.createTexture).toHaveBeenCalledOnce();
  renderer!.dispose();
  expect(native.unsubscribe).toHaveBeenCalledOnce();
});

it('settles after the idle budget, redraws theme/size once and wakes on interaction', () => {
  start();
  vi.advanceTimersByTime(188_000);
  const count = draws();
  expect(vi.getTimerCount()).toBe(0);
  vi.advanceTimersByTime(60_000);
  expect(draws()).toBe(count);
  mutation();
  vi.advanceTimersByTime(1000);
  expect(draws()).toBe(count + 1);
  expect(vi.getTimerCount()).toBe(0);
  rect.width = 1100;
  resize();
  vi.advanceTimersByTime(1000);
  expect(draws()).toBe(count + 2);
  win.dispatchEvent(new Event('keydown'));
  vi.advanceTimersByTime(1000);
  expect(draws()).toBeGreaterThan(count + 3);
  expect(gl.createProgram).toHaveBeenCalledOnce();
});

it.each(['gpu', 'slow', 'late'])('backs off %s failures and retries without rebuilding', (mode) => {
  start();
  gpuReady = mode !== 'gpu';
  cost = mode === 'slow' ? 70 : 0;
  late = mode === 'late' ? 20 : 0;
  vi.advanceTimersByTime(5000);
  const count = draws();
  vi.advanceTimersByTime(5000);
  expect(draws()).toBe(count);
  gpuReady = true;
  cost = late = 0;
  vi.advanceTimersByTime(60_000);
  expect(draws()).toBeGreaterThan(count);
  expect(gl.createProgram).toHaveBeenCalledOnce();
});

it('does not stack scheduled frames when themes change before a pending timer', () => {
  start();
  for (let index = 0; index < 20; index++) {
    vi.advanceTimersByTime(35);
    mutation();
    vi.advanceTimersByTime(25);
    expect(vi.getTimerCount()).toBe(1);
  }
  renderer!.dispose();
  const count = draws();
  vi.advanceTimersByTime(600_000);
  expect(draws()).toBe(count);
  expect(vi.getTimerCount()).toBe(0);
  expect(gl.deleteProgram).toHaveBeenCalledOnce();
  expect(gl.deleteTexture).toHaveBeenCalledOnce();
});

it('rejects software rendering before allocating shaders', () => {
  software = true;
  renderer = createCharacterRenderer(canvas);
  expect(renderer).toBeNull();
  expect(gl.createShader).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
});

it('stops on document hide and releases resources on context loss', () => {
  start();
  const count = draws();
  doc.visibilityState = 'hidden';
  doc.dispatchEvent(new Event('visibilitychange'));
  vi.advanceTimersByTime(1000);
  expect(draws()).toBe(count);
  doc.visibilityState = 'visible';
  doc.dispatchEvent(new Event('visibilitychange'));
  vi.advanceTimersByTime(1000);
  expect(draws()).toBeGreaterThan(count);
  canvas.dispatchEvent(new Event('webglcontextlost', { cancelable: true }));
  expect(vi.getTimerCount()).toBe(0);
  win.dispatchEvent(new Event('keydown'));
  vi.advanceTimersByTime(1000);
  expect(vi.getTimerCount()).toBe(0);
});
