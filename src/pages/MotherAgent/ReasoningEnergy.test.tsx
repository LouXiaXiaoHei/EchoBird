import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ReasoningEnergy } from './ReasoningEnergy';
import { createReasoningEnergyRenderer } from './reasoningEnergyRenderer';

vi.mock('./reasoningEnergyRenderer', () => ({ createReasoningEnergyRenderer: vi.fn() }));

let view: ReactTestRenderer | undefined;
let doc: EventTarget & { visibilityState: string };
let preferences: Map<string, EventTarget & { matches: boolean }>;
let children: HTMLCanvasElement[];
let order: string[];
const setActive = vi.fn();
const dispose = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  view = undefined;
  children = [];
  order = [];
  preferences = new Map();
  doc = Object.assign(new EventTarget(), {
    visibilityState: 'visible',
    createElement: () => {
      const canvas = Object.assign(new EventTarget(), {
        className: '',
        getContext: () => null,
        remove: () => {
          order.push('remove');
          children = children.filter((child) => child !== canvas);
        },
      }) as unknown as HTMLCanvasElement;
      return canvas;
    },
  });
  vi.stubGlobal('document', doc);
  vi.stubGlobal('window', {
    matchMedia: (query: string) => {
      if (!preferences.has(query)) {
        preferences.set(query, Object.assign(new EventTarget(), { matches: false }));
      }
      return preferences.get(query);
    },
  });
  dispose.mockImplementation(() => order.push('dispose'));
  vi.mocked(createReasoningEnergyRenderer).mockImplementation(() => ({ setActive, dispose }));
});

afterEach(() => {
  act(() => view?.unmount());
  vi.unstubAllGlobals();
});

function mount(active = true) {
  act(() => {
    if (view) view.update(<ReasoningEnergy active={active} />);
    else
      view = create(<ReasoningEnergy active={active} />, {
        createNodeMock: () => ({
          appendChild: (canvas: HTMLCanvasElement) => children.push(canvas),
        }),
      });
  });
}

it('starts only at max, releases into cooldown and reuses the same canvas until popup exit', () => {
  mount(false);
  expect(createReasoningEnergyRenderer).not.toHaveBeenCalled();
  mount(true);
  expect(createReasoningEnergyRenderer).toHaveBeenCalledOnce();
  expect(setActive).toHaveBeenLastCalledWith(true);
  mount(false);
  expect(setActive).toHaveBeenLastCalledWith(false);
  expect(dispose).not.toHaveBeenCalled();
  mount(true);
  expect(createReasoningEnergyRenderer).toHaveBeenCalledOnce();
  act(() => view?.unmount());
  expect(order).toEqual(['remove', 'dispose']);
});

it('detaches before disposal and creates a fresh canvas when the same host mounts again', () => {
  mount();
  const first = children[0];
  act(() => view?.unmount());
  view = undefined;
  mount();
  expect(children).toHaveLength(1);
  expect(children[0]).not.toBe(first);
  expect(order).toEqual(['remove', 'dispose']);
});

it('releases while hidden and under motion or contrast preferences, then restores the active state', () => {
  mount();
  act(() => {
    doc.visibilityState = 'hidden';
    doc.dispatchEvent(new Event('visibilitychange'));
  });
  expect(children).toHaveLength(0);
  act(() => {
    doc.visibilityState = 'visible';
    doc.dispatchEvent(new Event('visibilitychange'));
  });
  expect(children).toHaveLength(1);
  for (const preference of preferences.values()) {
    act(() => {
      preference.matches = true;
      preference.dispatchEvent(new Event('change'));
    });
    expect(children).toHaveLength(0);
    act(() => {
      preference.matches = false;
      preference.dispatchEvent(new Event('change'));
    });
    expect(children).toHaveLength(1);
    expect(setActive).toHaveBeenLastCalledWith(true);
  }
  act(() => view?.unmount());
  vi.mocked(createReasoningEnergyRenderer).mockClear();
  for (const preference of preferences.values()) preference.dispatchEvent(new Event('change'));
  expect(createReasoningEnergyRenderer).not.toHaveBeenCalled();
});

it('removes the failed canvas while leaving the slider decoration at rest', () => {
  vi.mocked(createReasoningEnergyRenderer).mockReturnValue(null);
  expect(mount).not.toThrow();
  expect(children).toHaveLength(0);
  expect(
    view!.root.findByProps({ className: 'mother-reasoning-energy' }).props['data-active']
  ).toBeUndefined();
});

it('hides a lost graphics context and disposes it only once', () => {
  mount();
  const event = new Event('webglcontextlost', { cancelable: true });
  act(() => {
    children[0].dispatchEvent(event);
  });
  expect(event.defaultPrevented).toBe(true);
  expect(children).toHaveLength(0);
  expect(dispose).toHaveBeenCalledOnce();
  act(() => view?.unmount());
  expect(dispose).toHaveBeenCalledOnce();
});
