import { useLayoutEffect, type ReactNode } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DndContext, PointerSensor } from '@dnd-kit/core';
import { ModelIdCombobox } from '../../components/ModelIdCombobox';
import * as api from '../../api/tauri';
import * as modelApi from '../../api/models';
import * as accountApi from '../../api/accountProxy';
import type { ModelConfig } from '../../api/types';
import { useAccountPrivacyStore } from '../../stores/accountPrivacyStore';
import { useNavigationStore } from '../../stores/navigationStore';
import {
  FreeModelsMain,
  FreeModelsProvider,
  FreeModelsTitleActions,
  useFreeModels,
} from './FreeModels';

const { confirm, showToast, edit, updateUserModel, drag } = vi.hoisted(() => ({
  confirm: vi.fn(),
  showToast: vi.fn(),
  edit: vi.fn(),
  updateUserModel: vi.fn(),
  drag: vi.fn(),
}));
vi.mock('../../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock('../../components', () => ({ getModelIcon: () => null }));
vi.mock('../../components/ConfirmDialog', () => ({ useConfirm: () => confirm }));
vi.mock('../../components/Toast', () => ({ useToast: () => ({ showToast }) }));
vi.mock('../ModelNexus/context', () => ({
  useModelNexus: () => ({ handleCardEdit: edit, updateUserModel }),
}));
vi.mock('react-dom', () => ({ createPortal: (children: ReactNode) => children }));
vi.mock('@dnd-kit/core', () => ({
  DndContext: ({ children }: { children: ReactNode }) => children,
  DragOverlay: ({ children }: { children: ReactNode }) => children,
  PointerSensor: vi.fn(),
  KeyboardSensor: vi.fn(),
  closestCenter: vi.fn(),
  pointerWithin: vi.fn(),
  rectIntersection: vi.fn(),
  useDroppable: () => ({ setNodeRef: vi.fn(), isOver: false }),
  useDndContext: () => ({ measureDroppableContainers: vi.fn() }),
  useSensor: (sensor: unknown, options: unknown) => ({ sensor, options }),
  useSensors: (...sensors: unknown[]) => sensors,
}));
vi.mock('@dnd-kit/sortable', async (original) => ({
  ...(await original<typeof import('@dnd-kit/sortable')>()),
  SortableContext: ({ children }: { children: ReactNode }) => children,
  useSortable: () => ({
    attributes: { tabIndex: 0 },
    listeners: { onPointerDown: drag },
    setNodeRef: vi.fn(),
    setActivatorNodeRef: vi.fn(),
    transform: null,
    transition: undefined,
    isDragging: false,
  }),
}));
vi.mock('../../api/models', () => ({
  getCachedUpstreamModelCatalog: vi.fn(),
  fetchUpstreamModelCatalog: vi.fn(),
}));
vi.mock('../../api/accountProxy', async (original) => ({
  ...(await original<typeof import('../../api/accountProxy')>()),
  listProxyAccounts: vi.fn().mockResolvedValue([
    {
      source: { provider: 'codex', accountId: 'account-one' },
      label: 'private@example.test',
      plan: 'Pro 100',
    },
  ]),
  getCachedAccountProxyModels: vi.fn().mockReturnValue(['account-old', 'account-new']),
  getAccountProxyModels: vi.fn(),
}));
vi.mock('../../api/tauri', () => ({
  getSmartRouterConfig: vi.fn(),
  getSmartRouterCandidates: vi.fn(),
  getSmartRouterActivity: vi.fn(),
  setSmartRouterEnabled: vi.fn(),
  setSmartRouterCandidateEnabled: vi.fn(),
  setSmartRouterCandidates: vi.fn(),
  removeSmartRouterCandidate: vi.fn(),
  getFreeModelDirectory: vi.fn(),
  getCachedFreeModelDirectory: vi.fn(),
  updateModel: vi.fn(),
  deleteModel: vi.fn(),
  queryModelUsage: vi.fn(),
}));

describe('Smart Router list layout and candidate pause', () => {
  let renderer: ReactTestRenderer;
  let context: ReturnType<typeof useFreeModels>;
  let models: ModelConfig[];
  let order: string[];
  let paused: string[];
  let storage: Map<string, string>;
  function config() {
    return {
      enabled: true,
      running: true,
      candidateIds: [...order],
      disabledCandidateIds: [...paused],
      usableCandidateCount: order.length - paused.length,
      baseUrl: 'http://127.0.0.1:53683/v1',
      modelId: 'auto',
      port: 53683,
    };
  }
  function Harness() {
    const value = useFreeModels();
    const activePage = useNavigationStore((state) => state.activePage);
    useLayoutEffect(() => {
      context = value;
    });
    return (
      <>
        <header>
          <FreeModelsTitleActions />
        </header>
        {activePage === 'freeModels' && <FreeModelsMain />}
      </>
    );
  }
  async function open() {
    await act(async () => {
      renderer = create(
        <FreeModelsProvider>
          <Harness />
        </FreeModelsProvider>
      );
    });
  }
  async function layout(mode: 'cards' | 'list') {
    const button = renderer.root.findByProps({ 'aria-label': `freeModels.layout.${mode}` });
    await act(async () => button.props.onClick());
  }
  function rows() {
    return renderer.root.findAll(
      (node) =>
        node.type === 'div' && String(node.props.className).startsWith('free-model-route-row ')
    );
  }
  function switches() {
    return renderer.root.findAllByProps({ role: 'switch' });
  }
  function buttons(prefix: string) {
    return renderer.root
      .findAllByType('button')
      .filter((node) => String(node.props['aria-label']).startsWith(prefix));
  }
  function noModelWrites() {
    expect(api.updateModel).not.toHaveBeenCalled();
    expect(api.deleteModel).not.toHaveBeenCalled();
    expect(api.setSmartRouterEnabled).not.toHaveBeenCalled();
    expect(api.queryModelUsage).not.toHaveBeenCalled();
  }
  beforeEach(() => {
    models = [
      {
        internalId: 'one',
        name: 'Saved provider',
        modelId: 'saved-model',
        baseUrl: 'https://saved.example/v1',
        apiKey: 'stored-key',
      },
      {
        internalId: 'two',
        name: 'Account provider',
        modelId: 'account-old',
        baseUrl: '',
        apiKey: '',
        accountSource: { provider: 'codex', accountId: 'account-one' },
        scope: 'smartRouter',
      },
      {
        internalId: 'three',
        name: 'Other provider',
        modelId: 'other-model',
        baseUrl: 'https://other.example/v1',
        apiKey: 'stored-key',
      },
    ];
    order = models.map((model) => model.internalId);
    paused = [];
    storage = new Map();
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
    });
    vi.stubGlobal('window', { setInterval, clearInterval, setTimeout });
    vi.stubGlobal(
      'document',
      Object.assign(new EventTarget(), { body: {}, visibilityState: 'visible' })
    );
    vi.stubGlobal('requestAnimationFrame', () => 1);
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    useAccountPrivacyStore.setState({ hidden: false });
    useNavigationStore.setState({ activePage: 'freeModels' });
    vi.mocked(api.getSmartRouterConfig).mockImplementation(async () => config());
    vi.mocked(api.getSmartRouterCandidates).mockImplementation(async () =>
      models.filter((model) => order.includes(model.internalId))
    );
    vi.mocked(api.getCachedFreeModelDirectory).mockResolvedValue(null);
    vi.mocked(api.getSmartRouterActivity).mockResolvedValue({
      candidateId: null,
      active: false,
      activeCandidateIds: [],
      sequence: 0,
      updatedAtMs: 0,
    });
    vi.mocked(modelApi.getCachedUpstreamModelCatalog).mockResolvedValue(null);
    vi.mocked(accountApi.listProxyAccounts).mockResolvedValue([
      {
        source: { provider: 'codex', accountId: 'account-one' },
        label: 'private@example.test',
        plan: 'Pro 100',
      },
    ]);
    vi.mocked(accountApi.getCachedAccountProxyModels).mockReturnValue([
      'account-old',
      'account-new',
    ]);
    vi.mocked(api.setSmartRouterCandidateEnabled).mockImplementation(async (id, enabled) => {
      paused = paused.filter((entry) => entry !== id);
      if (!enabled) paused.push(id);
      return config();
    });
    vi.mocked(api.setSmartRouterCandidates).mockImplementation(async (ids) => {
      order = [...ids];
      return config();
    });
    vi.mocked(api.removeSmartRouterCandidate).mockImplementation(async (id) => {
      order = order.filter((entry) => entry !== id);
      paused = paused.filter((entry) => entry !== id);
      return config();
    });
    confirm.mockResolvedValue(true);
    edit.mockResolvedValue(undefined);
  });
  afterEach(() => {
    act(() => renderer?.unmount());
    vi.resetAllMocks();
    vi.restoreAllMocks();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('places the only icon layout switch between privacy and help in the title bar', async () => {
    await open();
    const group = renderer.root.findByProps({ 'aria-label': 'freeModels.layout.label' });
    const titleGroups = renderer.root.findByType('header').findAllByProps({ role: 'group' });
    expect(titleGroups).toHaveLength(1);
    expect(titleGroups[0]).toBe(group);
    const controls = renderer.root.findByType('header').findAllByType('button');
    expect(controls.map((node) => node.props['aria-label'] ?? node.children[1])).toEqual([
      'freeModels.hideAccountNames',
      'freeModels.layout.cards',
      'freeModels.layout.list',
      'freeModels.help',
    ]);
    expect(group.findAllByType('svg')).toHaveLength(2);
    expect(group.findAllByType('button').map((node) => node.props['aria-pressed'])).toEqual([
      true,
      false,
    ]);
    expect(JSON.stringify(renderer.toJSON())).not.toMatch(/freeModels\.(view|super)\./);
  });

  it('defaults to cards and switches only presentation without fetching or changing routing', async () => {
    await open();
    expect(context.layout).toBe('cards');
    const reads = [
      api.getSmartRouterConfig,
      api.getSmartRouterCandidates,
      api.getSmartRouterActivity,
      modelApi.getCachedUpstreamModelCatalog,
      accountApi.listProxyAccounts,
    ].map((fn) => [fn, vi.mocked(fn).mock.calls.length] as const);
    const saved = context.customModels;
    await layout('list');
    expect(rows()).toHaveLength(3);
    expect(
      renderer.root.findAll(
        (node) => node.type === 'svg' && String(node.props.className).includes('absolute inset-0')
      )
    ).toHaveLength(0);
    await layout('cards');
    await layout('list');
    expect(context.customModels).toBe(saved);
    expect([...context.selectedIds]).toEqual(order);
    expect(context.routerEnabled).toBe(true);
    for (const [fn, count] of reads) expect(fn).toHaveBeenCalledTimes(count);
    expect(api.setSmartRouterCandidateEnabled).not.toHaveBeenCalled();
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
    expect(modelApi.fetchUpstreamModelCatalog).not.toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();
    expect(storage.get('echobird-router-layout')).toBe('list');
    noModelWrites();
  });

  it('pauses activity polling while hidden and resumes without changing routing', async () => {
    vi.useFakeTimers();
    vi.stubGlobal('window', { setInterval, clearInterval, setTimeout });
    await open();
    expect(api.getSmartRouterActivity).toHaveBeenCalledOnce();
    await act(async () => {
      Object.assign(document, { visibilityState: 'hidden' });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await act(async () => {
      vi.advanceTimersByTime(3000);
    });
    expect(api.getSmartRouterActivity).toHaveBeenCalledOnce();
    await act(async () => {
      Object.assign(document, { visibilityState: 'visible' });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    expect(api.getSmartRouterActivity).toHaveBeenCalledTimes(2);
    await act(async () => {
      vi.advanceTimersByTime(750);
    });
    expect(api.getSmartRouterActivity).toHaveBeenCalledTimes(3);
    expect(api.setSmartRouterEnabled).not.toHaveBeenCalled();
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    expect(accountApi.getAccountProxyModels).not.toHaveBeenCalled();
  });

  it('keeps activity polling across layouts and stops it on leaving the page', async () => {
    vi.useFakeTimers();
    vi.stubGlobal('window', { setInterval, clearInterval, setTimeout });
    await open();
    expect(api.getSmartRouterActivity).toHaveBeenCalledOnce();
    await layout('list');
    expect(api.getSmartRouterActivity).toHaveBeenCalledOnce();
    await act(async () => {
      vi.advanceTimersByTime(750);
    });
    expect(api.getSmartRouterActivity).toHaveBeenCalledTimes(2);
    await layout('cards');
    await act(async () => {
      vi.advanceTimersByTime(750);
    });
    expect(api.getSmartRouterActivity).toHaveBeenCalledTimes(3);
    await act(async () => useNavigationStore.getState().setActivePage('models'));
    await act(async () => {
      vi.advanceTimersByTime(1500);
    });
    expect(api.getSmartRouterActivity).toHaveBeenCalledTimes(3);
    await act(async () => useNavigationStore.getState().setActivePage('freeModels'));
    expect(api.getSmartRouterActivity).toHaveBeenCalledTimes(4);
    expect(api.setSmartRouterCandidateEnabled).not.toHaveBeenCalled();
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
    expect(accountApi.getAccountProxyModels).not.toHaveBeenCalled();
    expect(modelApi.fetchUpstreamModelCatalog).not.toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();
    expect(showToast).not.toHaveBeenCalled();
    noModelWrites();
  });

  it('disables the title layout switch during dragging and restores it after end, cancellation or leaving', async () => {
    await open();
    const control = () => renderer.root.findByProps({ 'aria-label': 'freeModels.layout.list' });
    const start = () =>
      act(() => renderer.root.findByType(DndContext).props.onDragStart({ active: { id: 'one' } }));
    start();
    expect(control().props.disabled).toBe(true);
    act(() => renderer.root.findByType(DndContext).props.onDragCancel());
    expect(control().props.disabled).toBe(false);
    start();
    act(() => renderer.root.findByType(DndContext).props.onDragEnd({ active: { id: 'one' } }));
    expect(control().props.disabled).toBe(false);
    start();
    await act(async () => useNavigationStore.getState().setActivePage('models'));
    expect(control().props.disabled).toBe(false);
    await act(async () => useNavigationStore.getState().setActivePage('freeModels'));
    await layout('list');
    expect(rows()).toHaveLength(3);
    expect(api.setSmartRouterCandidateEnabled).not.toHaveBeenCalled();
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
    noModelWrites();
  });

  it('remembers the list layout on remount and treats an unknown stored layout as cards', async () => {
    storage.set('echobird-router-layout', 'list');
    await open();
    expect(rows()).toHaveLength(3);
    act(() => renderer.unmount());
    storage.set('echobird-router-layout', 'invalid');
    await open();
    expect(context.layout).toBe('cards');
    expect(storage.get('echobird-router-layout')).toBe('invalid');
    noModelWrites();
  });

  it('loads paused entries, retains their priority across layouts, and restores them after remount', async () => {
    paused = ['one'];
    await open();
    await layout('list');
    expect(switches().map((node) => node.props['aria-checked'])).toEqual([false, true, true]);
    expect(JSON.stringify(renderer.toJSON())).toContain('freeModels.router.paused');
    await layout('cards');
    expect([...context.selectedIds]).toEqual(['one', 'two', 'three']);
    expect(
      renderer.root.findAll(
        (node) =>
          node.type === 'div' &&
          String(node.props.className).includes('free-model-route-node') &&
          String(node.props.className).includes('is-paused')
      )
    ).toHaveLength(1);
    act(() => renderer.unmount());
    await open();
    expect(context.pausedIds.has('one')).toBe(true);
    expect(api.setSmartRouterCandidateEnabled).not.toHaveBeenCalled();
  });

  it('pauses and resumes without deleting, moving or changing a model', async () => {
    await open();
    await layout('list');
    await act(async () => switches()[0].props.onClick());
    expect(api.setSmartRouterCandidateEnabled).toHaveBeenCalledExactlyOnceWith('one', false);
    expect(switches()[0].props['aria-checked']).toBe(false);
    await act(async () => switches()[0].props.onClick());
    expect(api.setSmartRouterCandidateEnabled).toHaveBeenLastCalledWith('one', true);
    expect([...context.selectedIds]).toEqual(['one', 'two', 'three']);
    expect(context.customModels).toHaveLength(3);
    expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
    noModelWrites();
  });

  it('toggles from the whole row without changing independent model controls', async () => {
    await open();
    await layout('list');
    const press = () => buttons('freeModels.router.priority ')[0];
    await act(async () => {
      press().props.onPointerDown({ clientX: 10, clientY: 10 });
      press().props.onClick({ detail: 1 });
    });
    expect(api.setSmartRouterCandidateEnabled).toHaveBeenCalledExactlyOnceWith('one', false);
    expect(press().props['aria-pressed']).toBe(false);
    expect(press().props.className).toContain('absolute inset-0');
    const preventDefault = vi.fn();
    await act(async () => press().props.onKeyDown({ key: 'Enter', preventDefault }));
    expect(preventDefault).toHaveBeenCalledOnce();
    expect(api.setSmartRouterCandidateEnabled).toHaveBeenLastCalledWith('one', true);
    const picker = renderer.root.findAllByType(ModelIdCombobox)[0];
    const stopPropagation = vi.fn();
    drag.mockClear();
    const pointer = {
      stopPropagation,
      target: {},
      currentTarget: { contains: () => true },
    };
    picker.parent!.props.onPointerDown(pointer);
    expect(stopPropagation).toHaveBeenCalledOnce();
    expect(drag).toHaveBeenCalledExactlyOnceWith(pointer);
    drag.mockClear();
    picker.parent!.props.onPointerDown({
      ...pointer,
      currentTarget: { contains: () => false },
    });
    expect(drag).not.toHaveBeenCalled();
    expect(picker.props.externalTriggerRef).toBeUndefined();
    expect(picker.props.open).toBe(false);
    expect(api.setSmartRouterCandidateEnabled).toHaveBeenCalledTimes(2);
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    noModelWrites();
  });

  it('starts dragging on movement without waiting and constrains only the list to vertical movement', async () => {
    await open();
    expect(renderer.root.findByType(DndContext).props.modifiers).toBeUndefined();
    await layout('list');
    const dnd = renderer.root.findByType(DndContext);
    expect(dnd.props.modifiers[0]({ transform: { x: 70, y: 90, scaleX: 1, scaleY: 1 } })).toEqual({
      x: 0,
      y: 90,
      scaleX: 1,
      scaleY: 1,
    });
    const descriptor = dnd.props.sensors.find(
      (sensor: { sensor: unknown }) => sensor.sensor === PointerSensor
    );
    const actual = await vi.importActual<typeof import('@dnd-kit/core')>('@dnd-kit/core');
    vi.useFakeTimers();
    try {
      const sensorDocument = Object.assign(new EventTarget(), { getSelection: () => null });
      vi.stubGlobal('document', sensorDocument);
      vi.stubGlobal('window', new EventTarget());
      const event = Object.assign(new Event('pointerdown'), { clientX: 10, clientY: 10 });
      Object.defineProperty(event, 'target', { value: new EventTarget() });
      type Props = ConstructorParameters<typeof actual.PointerSensor>[0];
      const onStart = vi.fn();
      const onMove = vi.fn();
      const onEnd = vi.fn();
      const onCancel = vi.fn();
      new actual.PointerSensor({
        active: 'one',
        activeNode: {} as Props['activeNode'],
        context: { current: {} as Props['context']['current'] },
        event,
        options: descriptor.options,
        onStart,
        onMove,
        onEnd,
        onCancel,
        onPending: vi.fn(),
        onAbort: vi.fn(),
      });
      sensorDocument.dispatchEvent(
        Object.assign(new Event('pointermove'), { clientX: 13, clientY: 16 })
      );
      expect(onStart).toHaveBeenCalledExactlyOnceWith({ x: 10, y: 10 });
      vi.advanceTimersByTime(300);
      expect(onCancel).not.toHaveBeenCalled();
      expect(onStart).toHaveBeenCalledExactlyOnceWith({ x: 10, y: 10 });
      sensorDocument.dispatchEvent(
        Object.assign(new Event('pointermove', { cancelable: true }), {
          clientX: 13,
          clientY: 100,
        })
      );
      expect(onMove).toHaveBeenCalledExactlyOnceWith({ x: 13, y: 100 });
      sensorDocument.dispatchEvent(new Event('pointerup'));
      expect(onEnd).toHaveBeenCalledOnce();
      vi.runOnlyPendingTimers();
    } finally {
      vi.useRealTimers();
    }
    noModelWrites();
  });

  it('shows the saved router status and both endpoint copy actions in the list hub bar', async () => {
    vi.mocked(api.getSmartRouterConfig).mockResolvedValueOnce({ ...config(), enabled: false });
    await open();
    await layout('list');
    const hub = renderer.root.find(
      (node) =>
        node.type === 'div' && String(node.props.className).includes('free-model-router-hub ')
    );
    expect(hub.props.className).toContain('free-model-router-hub-bar');
    expect(hub.findAll((node) => node.children.includes('freeModels.router.title'))).toHaveLength(
      0
    );
    expect(
      hub.findAll(
        (node) => node.type === 'span' && node.children.includes('freeModels.router.disabled')
      )
    ).toHaveLength(1);
    expect(buttons('btn.copy ')).toHaveLength(2);
    expect(
      hub.findAll((node) => node.type === 'span' && node.children.includes('127.0.0.1:53683/v1'))
    ).toHaveLength(1);
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    expect(api.setSmartRouterCandidateEnabled).not.toHaveBeenCalled();
    noModelWrites();
  });

  it('keeps the row disabled during a pending pause and preserves its state on failure', async () => {
    let reject!: (error: Error) => void;
    vi.mocked(api.setSmartRouterCandidateEnabled).mockImplementationOnce(
      () => new Promise((_, fail) => (reject = fail))
    );
    await open();
    await layout('list');
    await act(async () => buttons('freeModels.router.priority ')[0].props.onClick({ detail: 0 }));
    expect(buttons('freeModels.router.priority ')[0].props.disabled).toBe(true);
    await act(async () => buttons('freeModels.router.priority ')[0].props.onClick({ detail: 0 }));
    expect(api.setSmartRouterCandidateEnabled).toHaveBeenCalledOnce();
    await act(async () => reject(new Error('Save failed')));
    expect(buttons('freeModels.router.priority ')[0].props.disabled).toBe(false);
    expect(buttons('freeModels.router.priority ')[0].props['aria-pressed']).toBe(true);
    expect(switches()[0].props['aria-checked']).toBe(true);
    expect(showToast).toHaveBeenCalledExactlyOnceWith('error', 'error.requestFailed');
    noModelWrites();
  });

  it('does not mount the drag removal zone or removal hint in list mode', async () => {
    await open();
    await layout('list');
    await act(async () =>
      renderer.root.findByType(DndContext).props.onDragStart({ active: { id: 'one' } })
    );
    expect(
      renderer.root.findAll((node) =>
        String(node.props.className).includes('free-model-remove-zone')
      )
    ).toHaveLength(0);
    expect(JSON.stringify(renderer.toJSON())).not.toContain('freeModels.router.removeHintPrefix');
    await act(async () => renderer.root.findByType(DndContext).props.onDragCancel());
    expect(api.setSmartRouterCandidateEnabled).not.toHaveBeenCalled();
    expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
    noModelWrites();
  });

  it('locks a pending toggle and ignores duplicate changes before the first response', async () => {
    let resolve!: (value: ReturnType<typeof config>) => void;
    vi.mocked(api.setSmartRouterCandidateEnabled).mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        })
    );
    await open();
    await layout('list');
    await act(async () => {
      void context.setCandidateEnabled('one', false);
      void context.setCandidateEnabled('one', false);
    });
    expect(api.setSmartRouterCandidateEnabled).toHaveBeenCalledOnce();
    expect(switches()[0].props.disabled).toBe(true);
    expect(switches()[0].props['aria-checked']).toBe(true);
    expect(switches()[0].props['aria-busy']).toBe(true);
    await act(async () => resolve({ ...config(), disabledCandidateIds: ['one'] }));
    expect(switches()[0].props.disabled).toBe(false);
    expect(switches()[0].props['aria-checked']).toBe(false);
    noModelWrites();
  });

  it('keeps the saved switch state on failure and permits retry', async () => {
    vi.mocked(api.setSmartRouterCandidateEnabled).mockRejectedValueOnce(
      new Error('Disk unavailable')
    );
    await open();
    await layout('list');
    await act(async () => switches()[0].props.onClick());
    expect(switches()[0].props['aria-checked']).toBe(true);
    expect(switches()[0].props.disabled).toBe(false);
    expect(showToast).toHaveBeenCalledExactlyOnceWith('error', 'error.requestFailed');
    await act(async () => switches()[0].props.onClick());
    expect(switches()[0].props['aria-checked']).toBe(false);
    expect([...context.selectedIds]).toEqual(['one', 'two', 'three']);
  });

  it('keeps the global switch enabled when every entry is paused', async () => {
    await open();
    await act(async () => {
      for (const id of order) await context.setCandidateEnabled(id, false);
    });
    expect(context.pausedIds.size).toBe(3);
    expect(context.routerEnabled).toBe(true);
    expect([...context.selectedIds]).toEqual(['one', 'two', 'three']);
    expect(api.setSmartRouterEnabled).not.toHaveBeenCalled();
  });

  it('reorders the list without losing paused entries and rolls back failed reorders', async () => {
    paused = ['one'];
    await open();
    await layout('list');
    await act(async () =>
      renderer.root
        .findByType(DndContext)
        .props.onDragEnd({ active: { id: 'one' }, over: { id: 'three' } })
    );
    expect([...context.selectedIds]).toEqual(['two', 'three', 'one']);
    expect(context.pausedIds.has('one')).toBe(true);
    vi.mocked(api.setSmartRouterCandidates).mockRejectedValueOnce(new Error('Write failed'));
    await act(async () =>
      renderer.root
        .findByType(DndContext)
        .props.onDragEnd({ active: { id: 'one' }, over: { id: 'two' } })
    );
    expect([...context.selectedIds]).toEqual(['two', 'three', 'one']);
    expect(context.pausedIds.has('one')).toBe(true);
    expect(api.setSmartRouterCandidateEnabled).not.toHaveBeenCalled();
    noModelWrites();
  });

  it('uses confirmation to remove only the connection, preserving original models and accounts', async () => {
    paused = ['one'];
    confirm.mockResolvedValueOnce(false);
    await open();
    await layout('list');
    await act(async () => buttons('btn.remove:')[0].props.onClick());
    expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
    await act(async () => buttons('btn.remove:')[0].props.onClick());
    expect(api.removeSmartRouterCandidate).toHaveBeenCalledExactlyOnceWith('one');
    expect([...context.selectedIds]).toEqual(['two', 'three']);
    expect(context.pausedIds.has('one')).toBe(false);
    expect(models).toHaveLength(3);
    noModelWrites();
  });

  it('opens the existing editor for the intended model without saving, removing or dragging', async () => {
    await open();
    await layout('list');
    await act(async () => buttons('btn.edit:')[1].props.onClick());
    expect(edit).toHaveBeenCalledExactlyOnceWith(models[1]);
    expect(drag).not.toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();
    expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
    noModelWrites();
  });

  it('shows all actually working candidates and hides account identity while retaining its plan', async () => {
    paused = ['three'];
    vi.mocked(api.getSmartRouterActivity).mockResolvedValue({
      candidateId: 'two',
      active: true,
      activeCandidateIds: ['one', 'two'],
      sequence: 2,
      updatedAtMs: Date.now(),
    });
    useAccountPrivacyStore.setState({ hidden: true });
    await open();
    await layout('list');
    const text = JSON.stringify(renderer.toJSON());
    expect(text).not.toContain('private@example.test');
    expect(text).toContain('******');
    expect(text).toContain('Pro 100');
    expect(
      rows().filter((node) => String(node.props.className).includes('is-active'))
    ).toHaveLength(2);
    expect(rows()[2].props.className).toContain('is-paused');
    expect(renderer.root.findAllByType(ModelIdCombobox)).toHaveLength(3);
    noModelWrites();
  });
});
