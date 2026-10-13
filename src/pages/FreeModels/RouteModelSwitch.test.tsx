import { useLayoutEffect, type ComponentProps, type ReactNode } from 'react';
import { act, create, type ReactTestInstance, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DndContext, PointerSensor, useSensor, useSensors } from '@dnd-kit/core';
import * as api from '../../api/tauri';
import * as accountApi from '../../api/accountProxy';
import type { ModelConfig } from '../../api/types';
import type { FreeModelDirectory, FreeModelEntry } from '../../api/freeModels';
import { ModelIdCombobox } from '../../components/ModelIdCombobox';
import { useNavigationStore } from '../../stores/navigationStore';
import { FreeModelsMain, FreeModelsProvider, useFreeModels } from './FreeModels';

const { confirm, showToast, nexus, startDrag, keyboardDrag, dragState, measure, getModelIcon } =
  vi.hoisted(() => ({
    confirm: vi.fn(),
    showToast: vi.fn(),
    startDrag: vi.fn(),
    keyboardDrag: vi.fn(),
    dragState: { id: null as string | null },
    measure: vi.fn(),
    getModelIcon: vi.fn(),
    nexus: {
      updateUserModel: vi.fn(),
      setNewModelForm: vi.fn(),
      setEditingModelId: vi.fn(),
      setModelModalDestination: vi.fn(),
      setShowAddModelModal: vi.fn(),
      setKeyDestroyed: vi.fn(),
      setShowApiKey: vi.fn(),
    },
  }));
vi.mock('../../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock('../../components', () => ({ getModelIcon }));
vi.mock('../../components/ConfirmDialog', () => ({ useConfirm: () => confirm }));
vi.mock('../../components/Toast', () => ({ useToast: () => ({ showToast }) }));
vi.mock('../ModelNexus/context', () => ({ useModelNexus: () => nexus }));
vi.mock('react-dom', () => ({ createPortal: (children: ReactNode) => children }));
vi.mock('../../components/ModelIdCombobox', async (original) => {
  const actual = await original<typeof import('../../components/ModelIdCombobox')>();
  return {
    ModelIdCombobox: (props: ComponentProps<typeof actual.ModelIdCombobox>) =>
      props.optionLabels ? (
        <actual.ModelIdCombobox {...props} />
      ) : (
        <input value={props.value} disabled={props.disabled} readOnly />
      ),
  };
});
vi.mock('@dnd-kit/core', () => ({
  DndContext: ({ children }: { children: ReactNode }) => children,
  DragOverlay: ({ children }: { children: ReactNode }) => children,
  PointerSensor: vi.fn(),
  KeyboardSensor: vi.fn(),
  closestCenter: vi.fn(),
  pointerWithin: vi.fn(),
  rectIntersection: vi.fn(),
  useDroppable: () => ({ setNodeRef: vi.fn(), isOver: false }),
  useDndContext: () => ({ measureDroppableContainers: measure }),
  useSensor: vi.fn(),
  useSensors: vi.fn(),
}));
vi.mock('@dnd-kit/sortable', async (original) => ({
  ...(await original<typeof import('@dnd-kit/sortable')>()),
  SortableContext: ({ children }: { children: ReactNode }) => children,
  useSortable: ({ id }: { id: string }) => ({
    attributes: { tabIndex: 0 },
    listeners: { onPointerDown: startDrag, onKeyDown: keyboardDrag },
    setNodeRef: vi.fn(),
    setActivatorNodeRef: vi.fn(),
    transform: null,
    transition: undefined,
    isDragging: dragState.id === id,
  }),
}));
vi.mock('../../api/accountProxy', async (original) => ({
  ...(await original<typeof import('../../api/accountProxy')>()),
  getCachedAccountProxyModels: vi.fn(),
  getAccountProxyModels: vi.fn(),
  listProxyAccounts: vi.fn(),
}));
vi.mock('../../api/tauri', () => ({
  getSmartRouterConfig: vi.fn(),
  getSmartRouterCandidates: vi.fn(),
  getSmartRouterActivity: vi.fn(),
  setSmartRouterEnabled: vi.fn(),
  setSmartRouterCandidates: vi.fn(),
  removeSmartRouterCandidate: vi.fn(),
  getFreeModelDirectory: vi.fn(),
  getCachedFreeModelDirectory: vi.fn(),
  updateModel: vi.fn(),
  deleteModel: vi.fn(),
  queryModelUsage: vi.fn(),
  isKeyDestroyed: vi.fn(),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function freeEntry(modelId: string, baseUrl = 'https://free.example/v1'): FreeModelEntry {
  return {
    id: modelId,
    modelId,
    baseUrl,
    providerId: 'free-provider',
    provider: 'Free provider',
    freeType: 'perpetual',
    freeTier: 'Free',
    rateLimits: '',
    notes: '',
    docsUrl: '',
    cardRequired: null,
    phoneRequired: null,
    commercialOk: null,
    verifiedAt: '2026-10-12',
  };
}

describe('Direct route card model selection', () => {
  let renderer: ReactTestRenderer;
  let context: ReturnType<typeof useFreeModels>;
  let savedModels: ModelConfig[];
  let savedOrder: string[];
  const account: ModelConfig = {
    internalId: 'account-model',
    name: 'Account provider',
    modelId: 'account-old',
    baseUrl: 'https://account.example/v1',
    apiKey: '',
    accountSource: { provider: 'codex', accountId: 'account-one' },
    scope: 'smartRouter',
  };
  const free: ModelConfig = {
    internalId: 'free-model',
    name: 'Free provider',
    modelId: 'free-old',
    baseUrl: 'https://free.example/v1',
    anthropicUrl: 'https://free.example/anthropic',
    apiKey: 'saved-secret',
    scope: 'smartRouter',
  };
  const cached: FreeModelDirectory = {
    version: 1,
    updatedAt: '2026-10-12',
    models: [
      freeEntry('free-old'),
      freeEntry('free-new'),
      freeEntry('another-endpoint', 'https://free.example/v1/other'),
    ],
  };

  function config() {
    return {
      enabled: false,
      candidateIds: [...savedOrder],
      usableCandidateCount: savedOrder.length,
      baseUrl: 'http://127.0.0.1:62185/v1',
      modelId: 'smart-router',
      port: 62185,
      running: false,
    };
  }
  function Harness() {
    const value = useFreeModels();
    const activePage = useNavigationStore((state) => state.activePage);
    useLayoutEffect(() => {
      context = value;
    });
    return activePage === 'freeModels' ? <FreeModelsMain /> : null;
  }
  async function open() {
    await act(async () => {
      renderer = create(
        <FreeModelsProvider>
          <Harness />
        </FreeModelsProvider>,
        {
          createNodeMock: (element) =>
            String(element.props['aria-label']).startsWith('model.modelId')
              ? {
                  focus: vi.fn(),
                  blur: vi.fn(),
                  contains: () => false,
                  getBoundingClientRect: () => ({ left: 12, width: 160, top: 100, bottom: 130 }),
                }
              : null,
        }
      );
    });
  }
  function picker(modelId: string) {
    return renderer.root
      .findAllByType(ModelIdCombobox)
      .find((node) => node.props.value === modelId)!;
  }
  function card(modelId: string): ReactTestInstance {
    return renderer.root
      .findAll(
        (node) =>
          node.type === 'div' && String(node.props.className).includes('free-model-route-node')
      )
      .find((node) =>
        node.findAllByType(ModelIdCombobox).some((combo) => combo.props.value === modelId)
      )!;
  }
  function textOf(node: ReactTestInstance): string {
    return node.children
      .map((child) => (typeof child === 'string' ? child : textOf(child)))
      .join(' ');
  }
  function dragHandle(route: ReactTestInstance) {
    return route.findAllByType('button').find((node) => node.props.onPointerDown === startDrag)!;
  }
  function clickCard(modelId: string) {
    act(() => dragHandle(card(modelId)).props.onClick({ detail: 1 }));
  }
  function cardFor(modelId: string): ReactTestInstance {
    return renderer.root
      .findAll(
        (node) =>
          node.type === 'div' && String(node.props.className).includes('free-model-route-node')
      )
      .find((node) =>
        node
          .findAllByType('button')
          .some((button) => String(button.props['aria-label']).endsWith(`: ${modelId}`))
      )!;
  }
  async function select(oldId: string, newId: string) {
    await act(async () => {
      picker(oldId).props.onChange(newId);
    });
  }
  function expectNoPassiveChanges() {
    expect(api.getFreeModelDirectory).not.toHaveBeenCalled();
    expect(accountApi.getAccountProxyModels).not.toHaveBeenCalled();
    expect(api.queryModelUsage).not.toHaveBeenCalled();
    expect(api.updateModel).not.toHaveBeenCalled();
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
    expect(api.setSmartRouterEnabled).not.toHaveBeenCalled();
    expect(nexus.setShowAddModelModal).not.toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();
    expect(showToast).not.toHaveBeenCalled();
  }

  beforeEach(() => {
    dragState.id = null;
    vi.mocked(useSensor).mockImplementation((sensor, options) => ({
      sensor,
      options: options ?? {},
    }));
    vi.mocked(useSensors).mockImplementation((...sensors) =>
      sensors.filter((sensor) => sensor !== undefined && sensor !== null)
    );
    savedModels = [{ ...account }, { ...free }];
    savedOrder = savedModels.map((model) => model.internalId);
    getModelIcon.mockImplementation((name: string, modelId: string) =>
      modelId ? `/models/${modelId}.svg` : name === 'Free provider' ? '/providers/free.svg' : null
    );
    vi.stubGlobal('window', {
      setInterval,
      clearInterval,
      setTimeout,
      innerHeight: 800,
      innerWidth: 1000,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    });
    vi.stubGlobal('document', {
      body: {},
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    });
    vi.stubGlobal('requestAnimationFrame', () => 1);
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    useNavigationStore.setState({ activePage: 'freeModels' });
    vi.mocked(api.getSmartRouterConfig).mockImplementation(async () => config());
    vi.mocked(api.getSmartRouterCandidates).mockImplementation(async () => savedModels);
    vi.mocked(api.getSmartRouterActivity).mockResolvedValue({
      candidateId: null,
      active: false,
      sequence: 0,
      updatedAtMs: 0,
    });
    vi.mocked(api.getCachedFreeModelDirectory).mockResolvedValue(cached);
    vi.mocked(accountApi.getCachedAccountProxyModels).mockImplementation((source) =>
      source.accountId === 'account-one' ? ['account-old', 'account-new'] : undefined
    );
    vi.mocked(accountApi.listProxyAccounts).mockResolvedValue([
      {
        source: { provider: 'codex', accountId: 'account-one' },
        label: 'codex@example.com',
        plan: 'prolite',
      },
    ]);
    vi.mocked(api.updateModel).mockImplementation(async (id, updates) => {
      const existing = savedModels.find((model) => model.internalId === id);
      if (!existing) return null;
      const updated = { ...existing, ...updates };
      savedModels = savedModels.map((model) => (model.internalId === id ? updated : model));
      return updated;
    });
    vi.mocked(api.setSmartRouterCandidates).mockImplementation(async (ids) => {
      savedOrder = [...ids];
      return config();
    });
    vi.mocked(api.removeSmartRouterCandidate).mockImplementation(async (id) => {
      savedOrder = savedOrder.filter((candidate) => candidate !== id);
      return config();
    });
  });

  afterEach(() => {
    act(() => renderer?.unmount());
    vi.resetAllMocks();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('renders readable Qoder labels while keeping native selections and region caches unchanged', async () => {
    savedModels = [
      { ...account, modelId: 'cmodel', accountSource: { provider: 'qoder', accountId: 'shared' } },
      {
        ...account,
        internalId: 'cn-model',
        modelId: 'qmodel_38max',
        accountSource: { provider: 'qoderCn', accountId: 'shared' },
      },
    ];
    savedOrder = savedModels.map((model) => model.internalId);
    const original = structuredClone(savedModels);
    const writeCache = vi.spyOn(accountApi, 'cacheAccountProxyModels');
    vi.mocked(accountApi.getCachedAccountProxyModels).mockImplementation((source) =>
      source.provider === 'qoder' ? ['Cantus', 'Qwen3.8-Max'] : ['qmodel_38max', 'cn-other']
    );
    vi.mocked(accountApi.listProxyAccounts).mockImplementation(async (provider) => [
      {
        source: { provider: provider === 'qoder' ? 'qoderCn' : 'qoder', accountId: 'shared' },
        label: 'Other region',
        modelLabels: { cmodel: 'Wrong Region', qmodel_38max: 'Wrong Region' },
      },
      {
        source: { provider, accountId: 'shared' },
        label: `${provider} identity`,
        plan: 'free',
        modelLabels: {
          [provider === 'qoder' ? 'cmodel' : 'qmodel_38max']:
            provider === 'qoder' ? 'Cantus' : 'Qwen3.8-Max',
        },
      },
    ]);
    await open();
    expect(picker('cmodel').props.options).toEqual(['cmodel', 'Qwen3.8-Max']);
    expect(picker('qmodel_38max').props.options).toEqual(['qmodel_38max', 'cn-other']);
    expect(textOf(card('cmodel'))).toContain('Cantus');
    expect(textOf(card('cmodel'))).toContain('Free');
    expect(textOf(card('qmodel_38max'))).toContain('Qwen3.8-Max');
    expect(textOf(card('qmodel_38max'))).toContain('agent.qoderTrial');
    expect(textOf(card('cmodel'))).not.toContain('Wrong Region');
    clickCard('cmodel');
    const selected = renderer.root.find(
      (node) => node.type === 'div' && node.children[0] === 'Cantus' && node.props.onMouseDown
    );
    expect(selected.props.className).toContain('bg-cyber-text/15');
    await act(async () => selected.props.onMouseDown({ button: 0, preventDefault: vi.fn() }));
    expect(savedModels).toEqual(original);
    expect(writeCache).not.toHaveBeenCalled();
    expectNoPassiveChanges();

    const stopPropagation = vi.fn();
    act(() =>
      card('cmodel')
        .find((node) => String(node.props.className).includes('free-model-route-picker'))
        .props.onPointerDown({
          stopPropagation,
          target: {},
          currentTarget: { contains: () => true },
        })
    );
    expect(stopPropagation).toHaveBeenCalledOnce();
    expect(startDrag).toHaveBeenCalledOnce();
    await act(async () =>
      renderer.root.findByType(DndContext).props.onDragStart({ active: { id: account.internalId } })
    );
    const overlay = renderer.root.find(
      (node) =>
        node.type === 'div' && String(node.props.className).includes('is-dragging shadow-2xl')
    );
    expect(textOf(overlay)).toContain('Cantus');
    await act(async () => renderer.root.findByType(DndContext).props.onDragCancel());
    expectNoPassiveChanges();
    await select('cmodel', 'Qwen3.8-Max');
    expect(api.updateModel).toHaveBeenCalledExactlyOnceWith(account.internalId, {
      modelId: 'Qwen3.8-Max',
    });
  });

  it('keeps unknown native IDs readable as IDs without borrowing another account label', async () => {
    savedModels = [
      { ...account, modelId: 'cmodel', accountSource: { provider: 'qoder', accountId: 'missing' } },
    ];
    savedOrder = [account.internalId];
    vi.mocked(accountApi.getCachedAccountProxyModels).mockReturnValue(['cmodel', 'unknown-model']);
    vi.mocked(accountApi.listProxyAccounts).mockResolvedValue([
      {
        source: { provider: 'qoder', accountId: 'other' },
        label: 'Other account',
        modelLabels: { cmodel: 'Other Account Model' },
      },
    ]);
    await open();
    expect(picker('cmodel').props.options).toEqual(['cmodel', 'unknown-model']);
    expect(picker('cmodel').props.optionLabels).toBeUndefined();
    expect(textOf(card('cmodel'))).not.toContain('Other Account Model');
    expectNoPassiveChanges();
  });

  it('uses each account cache and the exact free endpoint without fetching, applying or opening an editor', async () => {
    savedModels.push(
      {
        ...account,
        internalId: 'other-account',
        modelId: 'other-old',
        accountSource: { provider: 'codex', accountId: 'account-two' },
      },
      {
        ...account,
        internalId: 'uncached-account',
        modelId: 'uncached',
        baseUrl: free.baseUrl,
        accountSource: { provider: 'codex', accountId: 'missing' },
      },
      { ...free, internalId: 'manual', modelId: 'manual-id', baseUrl: 'https://manual.example/v1' },
      { ...free, internalId: 'local-server', modelId: 'local-id' }
    );
    savedOrder = savedModels.map((model) => model.internalId);
    vi.mocked(accountApi.getCachedAccountProxyModels).mockImplementation((source) =>
      source.accountId === 'account-one'
        ? ['account-old', 'account-new']
        : source.accountId === 'account-two'
          ? ['other-old', 'other-new']
          : undefined
    );
    await open();
    expect(picker('account-old').props.options).toEqual(['account-old', 'account-new']);
    expect(picker('other-old').props.options).toEqual(['other-old', 'other-new']);
    expect(picker('free-old').props.options).toEqual(['free-old', 'free-new']);
    expect(renderer.root.findAllByType(ModelIdCombobox)).toHaveLength(3);
    for (const combo of renderer.root.findAllByType(ModelIdCombobox)) {
      expect(combo.props.selectOnly).toBe(true);
      expect(combo.props.compact).toBe(true);
      expect(combo.props.onRefresh).toBeUndefined();
    }
    expect(JSON.stringify(renderer.toJSON())).toContain('uncached');
    expect(JSON.stringify(renderer.toJSON())).toContain('manual-id');
    expect(JSON.stringify(renderer.toJSON())).toContain('local-id');
    expect(accountApi.listProxyAccounts).toHaveBeenCalledExactlyOnceWith('codex');
    await act(async () => useNavigationStore.getState().setActivePage('models'));
    await act(async () => useNavigationStore.getState().setActivePage('freeModels'));
    expect(vi.mocked(accountApi.listProxyAccounts).mock.calls).toEqual([['codex'], ['codex']]);
    expectNoPassiveChanges();
  });

  it('keeps the cached endpoint provider icon across model-family changes, drag previews and reloads', async () => {
    savedModels = [
      { ...free, name: 'Saved nickname', modelId: 'claude-old', scope: 'smartRouter' },
    ];
    savedOrder = [free.internalId];
    vi.mocked(api.getCachedFreeModelDirectory).mockResolvedValue({
      ...cached,
      models: [freeEntry('claude-old'), freeEntry('deepseek-new')],
    });
    await open();
    expect(card('claude-old').findByType('img').props.src).toBe('/providers/free.svg');
    expect(textOf(card('claude-old'))).toContain('Saved nickname');
    expectNoPassiveChanges();
    await select('claude-old', 'deepseek-new');
    expect(card('deepseek-new').findByType('img').props.src).toBe('/providers/free.svg');
    expect(api.updateModel).toHaveBeenCalledExactlyOnceWith(free.internalId, {
      modelId: 'deepseek-new',
    });
    await act(async () => {
      renderer.root.findByType(DndContext).props.onDragStart({ active: { id: free.internalId } });
    });
    const overlay = renderer.root.find(
      (node) =>
        node.type === 'div' &&
        node.props['aria-hidden'] === true &&
        String(node.props.className).includes('free-model-route-node')
    );
    expect(overlay.findByType('img').props.src).toBe('/providers/free.svg');
    await act(async () => renderer.root.findByType(DndContext).props.onDragCancel());
    act(() => renderer.unmount());
    await open();
    expect(card('deepseek-new').findByType('img').props.src).toBe('/providers/free.svg');
    expect(api.updateModel).toHaveBeenCalledOnce();
    expect(api.getFreeModelDirectory).not.toHaveBeenCalled();
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
  });

  it('preserves model-family icons for saved models on a free endpoint and tool icons for accounts', async () => {
    savedModels = [
      { ...free, modelId: 'claude-saved', scope: 'modelCenter' },
      { ...free, internalId: 'legacy-model', modelId: 'deepseek-legacy', scope: undefined },
      { ...account, modelId: 'account-old', baseUrl: free.baseUrl },
    ];
    savedOrder = savedModels.map((model) => model.internalId);
    await open();
    expect(card('claude-saved').findByType('img').props.src).toBe('/models/claude-saved.svg');
    expect(card('deepseek-legacy').findByType('img').props.src).toBe('/models/deepseek-legacy.svg');
    const codex = accountApi.accountProxyProviders.find((provider) => provider.id === 'codex')!;
    expect(card('account-old').findByType('img').props.src).toBe(`/icons/tools/${codex.icon}`);
    expectNoPassiveChanges();
  });

  it('retains the model-icon fallback for a missing free endpoint and the local server', async () => {
    savedModels = [
      {
        ...free,
        internalId: 'missing-provider',
        baseUrl: `${free.baseUrl}/other-missing`,
        modelId: 'claude-missing',
      },
      { ...free, internalId: 'local-server', modelId: 'deepseek-local' },
    ];
    savedOrder = savedModels.map((model) => model.internalId);
    await open();
    expect(cardFor('claude-missing').findByType('img').props.src).toBe(
      '/models/claude-missing.svg'
    );
    expect(cardFor('deepseek-local').findByType('img').props.src).toBe(
      '/models/deepseek-local.svg'
    );
    expectNoPassiveChanges();
  });

  it('shows the saved account identity and plan while preserving API names, credentials and source IDs', async () => {
    const buddy: ModelConfig = {
      ...account,
      internalId: 'buddy-model',
      name: 'Old account nickname',
      modelId: 'buddy-auto',
      accountSource: { provider: 'workBuddy', accountId: 'account-one' },
    };
    savedModels.push(buddy, {
      ...account,
      internalId: 'second-codex',
      modelId: 'second-old',
      accountSource: { provider: 'codex', accountId: 'account-two' },
    });
    savedOrder = savedModels.map((model) => model.internalId);
    vi.mocked(accountApi.getCachedAccountProxyModels).mockImplementation((source) =>
      source.provider === 'workBuddy'
        ? ['buddy-auto', 'buddy-new']
        : source.accountId === 'account-two'
          ? ['second-old', 'second-new']
          : ['account-old', 'account-new']
    );
    vi.mocked(accountApi.listProxyAccounts).mockImplementation(async (provider) =>
      provider === 'workBuddy'
        ? [
            {
              source: { provider: 'workBuddy', accountId: 'account-one' },
              label: 'Ben.',
              plan: '体验版',
            },
          ]
        : [
            {
              source: { provider: 'codex', accountId: 'account-one' },
              label: 'codex@example.com',
              plan: 'prolite',
            },
            {
              source: { provider: 'codex', accountId: 'account-two' },
              label: 'second@example.com',
              plan: 'team',
            },
          ]
    );
    const original = structuredClone(savedModels);
    await open();
    const titles = renderer.root
      .findAllByType('span')
      .filter((node) => String(node.props.className).includes('font-semibold'))
      .map((node) => node.children.join(''));
    expect(titles).toContain('codex@example.com');
    expect(titles).toContain('second@example.com');
    expect(titles).toContain('Ben.');
    expect(titles).toContain('Free provider');
    expect(titles).not.toContain('Account provider');
    expect(titles).not.toContain('Old account nickname');
    expect(accountApi.listProxyAccounts).toHaveBeenCalledTimes(2);
    expect(accountApi.listProxyAccounts).toHaveBeenCalledWith('codex');
    expect(accountApi.listProxyAccounts).toHaveBeenCalledWith('workBuddy');
    expect(textOf(card('account-old'))).toContain('Pro 100');
    expect(textOf(card('second-old'))).toContain('Business');
    expect(textOf(card('buddy-auto'))).toContain('体验版');
    for (const [modelId, plan] of [
      ['account-old', 'Pro 100'],
      ['second-old', 'Business'],
      ['buddy-auto', '体验版'],
    ]) {
      const tier = card(modelId)
        .findAllByType('span')
        .find((node) => node.children.join('').includes(plan))!;
      expect(tier.children.join('')).toBe(plan);
    }
    expect(savedModels).toEqual(original);
    expectNoPassiveChanges();
  });

  it('uses the provider name while account metadata is pending or missing without exposing internal IDs', async () => {
    const pending = deferred<accountApi.ProxyAccount[]>();
    vi.mocked(accountApi.listProxyAccounts).mockReturnValueOnce(pending.promise);
    await open();
    const titles = () =>
      renderer.root
        .findAllByType('span')
        .filter((node) => String(node.props.className).includes('font-semibold'))
        .map((node) => node.children.join(''));
    expect(titles()).toContain('ChatGPT / Codex');
    expect(titles()).not.toContain('Account provider');
    expect(titles().join(' ')).not.toContain('account-one');
    await act(async () => pending.resolve([]));
    expect(titles()).toContain('ChatGPT / Codex');
    expect(textOf(card('account-old'))).toContain('—');
    expect(titles().join(' ')).not.toContain('account-one');
    expectNoPassiveChanges();
  });

  it('keeps the loaded identity after a failed passive reread and refreshes it only on router entry', async () => {
    await open();
    expect(textOf(card('account-old'))).toContain('codex@example.com');
    expect(textOf(card('account-old'))).toContain('Pro 100');
    expect(accountApi.listProxyAccounts).toHaveBeenCalledExactlyOnceWith('codex');
    await select('account-old', 'account-new');
    expect(accountApi.listProxyAccounts).toHaveBeenCalledOnce();
    vi.mocked(api.updateModel).mockClear();
    vi.mocked(accountApi.listProxyAccounts).mockRejectedValueOnce(
      new Error('Local metadata unavailable')
    );
    await act(async () => useNavigationStore.getState().setActivePage('models'));
    expect(accountApi.listProxyAccounts).toHaveBeenCalledOnce();
    await act(async () => useNavigationStore.getState().setActivePage('freeModels'));
    expect(vi.mocked(accountApi.listProxyAccounts).mock.calls).toEqual([['codex'], ['codex']]);
    expect(textOf(card('account-new'))).toContain('codex@example.com');
    expect(textOf(card('account-new'))).toContain('Pro 100');
    expect(showToast).not.toHaveBeenCalled();
    expectNoPassiveChanges();
  });

  it('loads no account metadata outside the active smart view and ignores a late earlier entry', async () => {
    useNavigationStore.setState({ activePage: 'models' });
    await open();
    expect(accountApi.listProxyAccounts).not.toHaveBeenCalled();
    const earlier = deferred<accountApi.ProxyAccount[]>();
    vi.mocked(accountApi.listProxyAccounts).mockReturnValueOnce(earlier.promise);
    await act(async () => useNavigationStore.getState().setActivePage('freeModels'));
    expect(accountApi.listProxyAccounts).toHaveBeenCalledExactlyOnceWith('codex');
    await act(async () => context.setViewMode('super'));
    expect(accountApi.listProxyAccounts).toHaveBeenCalledOnce();
    await act(async () => context.setViewMode('smart'));
    expect(accountApi.listProxyAccounts).toHaveBeenCalledTimes(2);
    expect(textOf(card('account-old'))).toContain('codex@example.com');
    await act(async () =>
      earlier.resolve([
        {
          source: { provider: 'codex', accountId: 'account-one' },
          label: 'stale@example.com',
          plan: 'free',
        },
      ])
    );
    expect(textOf(card('account-old'))).toContain('codex@example.com');
    expect(textOf(card('account-old'))).not.toContain('stale@example.com');
    expectNoPassiveChanges();
  });

  it('persists only the model ID and synchronizes the route and saved-model view without changing priority or credentials', async () => {
    await open();
    await select('free-old', 'free-new');
    const updated = { ...free, modelId: 'free-new' };
    expect(api.updateModel).toHaveBeenCalledExactlyOnceWith(free.internalId, {
      modelId: 'free-new',
    });
    expect(context.customModels.find((model) => model.id === free.internalId)?.modelId).toBe(
      'free-new'
    );
    expect([...context.selectedIds]).toEqual(['account-model', 'free-model']);
    expect(nexus.updateUserModel).toHaveBeenCalledExactlyOnceWith(updated, {
      name: updated.name,
      baseUrl: updated.baseUrl,
      anthropicUrl: updated.anthropicUrl,
      apiKey: updated.apiKey,
      modelId: updated.modelId,
      accountSource: updated.accountSource,
    });
    expect(picker('free-new').props.disabled).toBe(false);
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    expect(nexus.setShowAddModelModal).not.toHaveBeenCalled();
    act(() => renderer.unmount());
    await open();
    expect(picker('free-new')).toBeDefined();
    expect(api.updateModel).toHaveBeenCalledOnce();
    expect(showToast).not.toHaveBeenCalled();
  });

  it('toggles cached model choices on repeated card clicks while static cards and drag previews have no menu action', async () => {
    savedModels.push({
      ...free,
      internalId: 'manual',
      modelId: 'manual-id',
      baseUrl: 'https://manual.example/v1',
    });
    savedOrder = savedModels.map((model) => model.internalId);
    await open();
    expect(picker('account-old').props.open).toBe(false);
    clickCard('account-old');
    expect(picker('account-old').props.open).toBe(true);
    expect(picker('free-old').props.open).toBe(false);
    clickCard('account-old');
    expect(picker('account-old').props.open).toBe(false);
    clickCard('account-old');
    expect(picker('account-old').props.open).toBe(true);
    act(() => dragHandle(cardFor('manual-id')).props.onClick?.({ detail: 1 }));
    expect(renderer.root.findAllByType(ModelIdCombobox)).toHaveLength(2);
    await act(async () =>
      renderer.root.findByType(DndContext).props.onDragStart({ active: { id: 'account-model' } })
    );
    const overlay = renderer.root.find(
      (node) =>
        node.type === 'div' &&
        node.props['aria-hidden'] === true &&
        String(node.props.className).includes('free-model-route-node')
    );
    expect(overlay.findAllByType(ModelIdCombobox)).toHaveLength(0);
    act(() => overlay.findByType('button').props.onClick?.({ detail: 1 }));
    expectNoPassiveChanges();
  });

  it('uses Enter to open the card menu while preserving Space for keyboard dragging', async () => {
    await open();
    const preventDefault = vi.fn();
    act(() =>
      dragHandle(card('account-old')).props.onKeyDown({
        key: 'Enter',
        code: 'Enter',
        preventDefault,
      })
    );
    expect(preventDefault).toHaveBeenCalledOnce();
    expect(picker('account-old').props.open).toBe(true);
    expect(keyboardDrag).not.toHaveBeenCalled();
    const space = { key: ' ', code: 'Space', preventDefault: vi.fn() };
    act(() => dragHandle(card('account-old')).props.onKeyDown(space));
    expect(keyboardDrag).toHaveBeenCalledExactlyOnceWith(space);
    expectNoPassiveChanges();
  });

  it('keeps the menu closed after an open card loses and regains its cached model options', async () => {
    await open();
    clickCard('account-old');
    expect(picker('account-old').props.open).toBe(true);
    vi.mocked(accountApi.getCachedAccountProxyModels).mockReturnValue(undefined);
    act(() =>
      renderer.update(
        <FreeModelsProvider>
          <Harness />
        </FreeModelsProvider>
      )
    );
    expect(cardFor('account-old').findAllByType(ModelIdCombobox)).toHaveLength(0);
    act(() => dragHandle(cardFor('account-old')).props.onClick({ detail: 1 }));
    vi.mocked(accountApi.getCachedAccountProxyModels).mockReturnValue([
      'account-old',
      'account-new',
    ]);
    act(() =>
      renderer.update(
        <FreeModelsProvider>
          <Harness />
        </FreeModelsProvider>
      )
    );
    expect(picker('account-old').props.open).toBe(false);
    clickCard('account-old');
    expect(picker('account-old').props.open).toBe(true);
    expectNoPassiveChanges();
  });

  it('closes the card menu for a model save and prevents clicks from reopening it while pending', async () => {
    const pending = deferred<ModelConfig | null>();
    vi.mocked(api.updateModel).mockReturnValue(pending.promise);
    await open();
    clickCard('account-old');
    expect(picker('account-old').props.open).toBe(true);
    await select('account-old', 'account-new');
    expect(picker('account-old').props.disabled).toBe(true);
    expect(picker('account-old').props.open).toBe(false);
    clickCard('account-old');
    expect(picker('account-old').props.open).toBe(false);
    await act(async () => pending.resolve({ ...account, modelId: 'account-new' }));
    expect(picker('account-new').props.open).toBe(false);
    expect(picker('account-new').props.disabled).toBe(false);
    expect(api.updateModel).toHaveBeenCalledExactlyOnceWith('account-model', {
      modelId: 'account-new',
    });
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
    expect(startDrag).not.toHaveBeenCalled();
  });

  it('closes and locks an open menu while the same card is dragged and keeps it closed after release', async () => {
    await open();
    clickCard('account-old');
    expect(picker('account-old').props.open).toBe(true);
    dragState.id = 'account-model';
    act(() =>
      renderer.update(
        <FreeModelsProvider>
          <Harness />
        </FreeModelsProvider>
      )
    );
    expect(picker('account-old').props.disabled).toBe(true);
    expect(picker('account-old').props.open).toBe(false);
    clickCard('account-old');
    expect(picker('account-old').props.open).toBe(false);
    dragState.id = null;
    act(() =>
      renderer.update(
        <FreeModelsProvider>
          <Harness />
        </FreeModelsProvider>
      )
    );
    expect(picker('account-old').props.disabled).toBe(false);
    expect(picker('account-old').props.open).toBe(false);
    clickCard('account-old');
    expect(picker('account-old').props.open).toBe(true);
    expectNoPassiveChanges();
  });

  it('ignores the current model, permits dragging from the model row, and excludes portalled options and keyboard selection', async () => {
    await open();
    const route = card('account-old');
    const handle = dragHandle(route);
    expect(handle).toBeDefined();
    handle.props.onPointerDown();
    expect(startDrag).toHaveBeenCalledOnce();
    startDrag.mockClear();
    const pickerWrapper = picker('account-old').parent!;
    const stopPointer = vi.fn();
    const stopKeyboard = vi.fn();
    const modelRowTarget = {};
    const rowPress = {
      target: modelRowTarget,
      currentTarget: { contains: (target: unknown) => target === modelRowTarget },
      stopPropagation: stopPointer,
    };
    pickerWrapper.props.onPointerDown(rowPress);
    expect(startDrag).toHaveBeenCalledExactlyOnceWith(rowPress);
    startDrag.mockClear();
    const menuPress = {
      target: {},
      currentTarget: rowPress.currentTarget,
      stopPropagation: vi.fn(),
    };
    pickerWrapper.props.onPointerDown(menuPress);
    expect(menuPress.stopPropagation).toHaveBeenCalledOnce();
    expect(startDrag).not.toHaveBeenCalled();
    pickerWrapper.props.onKeyDown({ key: ' ', stopPropagation: stopKeyboard });
    expect(stopPointer).toHaveBeenCalledOnce();
    expect(stopKeyboard).toHaveBeenCalledOnce();
    expect(keyboardDrag).not.toHaveBeenCalled();
    await select('account-old', 'account-old');
    await select('account-old', 'unlisted-model');
    expect(api.updateModel).not.toHaveBeenCalled();
    expect(startDrag).not.toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();
    expect(nexus.setShowAddModelModal).not.toHaveBeenCalled();
  });

  it('preserves quick clicks, held clicks, immediate dragging and hold-then-drag while suppressing drag-release clicks with the installed sensor', async () => {
    await open();
    const descriptor = renderer.root
      .findByType(DndContext)
      .props.sensors.find((sensor: { sensor: unknown }) => sensor.sensor === PointerSensor);
    const constraint = descriptor.options.activationConstraint;
    expect(constraint.distance).toBeGreaterThan(0);
    expect(constraint.delay).toBeUndefined();
    expect(constraint.tolerance).toBeUndefined();
    const actual = await vi.importActual<typeof import('@dnd-kit/core')>('@dnd-kit/core');
    vi.useFakeTimers();
    try {
      const getSelection = vi.fn(() => null);
      const sensorDocument = Object.assign(new EventTarget(), { getSelection });
      const sensorWindow = new EventTarget();
      vi.stubGlobal('document', sensorDocument);
      vi.stubGlobal('window', sensorWindow);
      expect(document).toBe(sensorDocument);
      const addListener = vi.spyOn(sensorDocument, 'addEventListener');
      const removeListener = vi.spyOn(sensorDocument, 'removeEventListener');
      const onStart = vi.fn();
      const onAbort = vi.fn();
      const onMove = vi.fn();
      const onEnd = vi.fn();
      const press = () => {
        const event = Object.assign(new Event('pointerdown'), { clientX: 10, clientY: 10 });
        Object.defineProperty(event, 'target', { value: new EventTarget() });
        type Props = ConstructorParameters<typeof actual.PointerSensor>[0];
        return new actual.PointerSensor({
          active: 'account-model',
          activeNode: {} as Props['activeNode'],
          context: { current: {} as Props['context']['current'] },
          event,
          options: descriptor.options,
          onStart,
          onAbort,
          onPending: vi.fn(),
          onCancel: vi.fn(),
          onMove,
          onEnd,
        });
      };
      for (const heldMs of [0, 1000]) {
        press();
        vi.advanceTimersByTime(heldMs);
        expect(onStart).not.toHaveBeenCalled();
        sensorDocument.dispatchEvent(new Event('pointerup'));
        vi.runOnlyPendingTimers();
      }
      expect(onStart).not.toHaveBeenCalled();
      expect(onAbort.mock.calls).toEqual([['account-model'], ['account-model']]);
      expect(addListener.mock.calls.some(([name]) => name === 'click')).toBe(false);

      onAbort.mockClear();
      onEnd.mockClear();
      press();
      vi.advanceTimersByTime(1000);
      expect(onStart).not.toHaveBeenCalled();
      sensorDocument.dispatchEvent(
        Object.assign(new Event('pointermove'), { clientX: 11, clientY: 10 })
      );
      expect(onStart).not.toHaveBeenCalled();
      sensorDocument.dispatchEvent(
        Object.assign(new Event('pointermove'), { clientX: 22, clientY: 10 })
      );
      expect(onStart).toHaveBeenCalledExactlyOnceWith({ x: 10, y: 10 });
      expect(getSelection).toHaveBeenCalledOnce();
      vi.advanceTimersByTime(1000);
      expect(onStart).toHaveBeenCalledOnce();
      sensorDocument.dispatchEvent(
        Object.assign(new Event('pointermove', { cancelable: true }), {
          clientX: 30,
          clientY: 10,
        })
      );
      expect(onMove).toHaveBeenCalledExactlyOnceWith({ x: 30, y: 10 });
      const capture = addListener.mock.calls.find(([name]) => name === 'click')!;
      expect(capture[2]).toEqual({ capture: true });
      const stopPropagation = vi.fn();
      (capture[1] as (event: { stopPropagation: () => void }) => void)({ stopPropagation });
      expect(stopPropagation).toHaveBeenCalledOnce();
      sensorDocument.dispatchEvent(new Event('pointerup'));
      expect(onAbort).not.toHaveBeenCalled();
      expect(onEnd).toHaveBeenCalledOnce();
      expect(removeListener.mock.calls.some(([name]) => name === 'click')).toBe(false);
      vi.advanceTimersByTime(50);
      expect(removeListener).toHaveBeenCalledWith('click', capture[1], capture[2]);

      onStart.mockClear();
      press();
      sensorDocument.dispatchEvent(
        Object.assign(new Event('pointermove'), {
          clientX: 22,
          clientY: 10,
        })
      );
      expect(onStart).toHaveBeenCalledExactlyOnceWith({ x: 10, y: 10 });
      expect(onAbort).not.toHaveBeenCalled();
      vi.advanceTimersByTime(1000);
      expect(onStart).toHaveBeenCalledOnce();
      sensorDocument.dispatchEvent(new Event('pointerup'));
      expect(onEnd).toHaveBeenCalledTimes(2);
      vi.runOnlyPendingTimers();
    } finally {
      vi.useRealTimers();
    }
  });

  it('blocks repeated selections and omits card actions while preserving the old model until persistence succeeds', async () => {
    const pending = deferred<ModelConfig | null>();
    vi.mocked(api.updateModel).mockReturnValue(pending.promise);
    await open();
    const selectModel = picker('account-old').props.onChange;
    await act(async () => {
      selectModel('account-new');
      selectModel('account-new');
    });
    expect(api.updateModel).toHaveBeenCalledExactlyOnceWith('account-model', {
      modelId: 'account-new',
    });
    expect(picker('account-old').props.disabled).toBe(true);
    expect(context.customModels.find((model) => model.id === 'account-model')?.modelId).toBe(
      'account-old'
    );
    const actions = card('account-old')
      .findAllByType('button')
      .filter((node) => String(node.props['aria-label']).startsWith('btn.'));
    expect(actions).toHaveLength(0);
    expect(confirm).not.toHaveBeenCalled();
    expect(nexus.setShowAddModelModal).not.toHaveBeenCalled();
    expect(picker('free-old').props.disabled).toBe(false);
    expect(nexus.updateUserModel).not.toHaveBeenCalled();
    await act(async () => pending.resolve({ ...account, modelId: 'account-new' }));
    expect(picker('account-new').props.disabled).toBe(false);
    expect(nexus.updateUserModel).toHaveBeenCalledOnce();
    expect([...context.selectedIds]).toEqual(['account-model', 'free-model']);
  });

  it.each(['presentation', 'page navigation'] as const)(
    'keeps a pending card locked across %s until persistence completes',
    async (exit) => {
      const pending = deferred<ModelConfig | null>();
      vi.mocked(api.updateModel).mockImplementationOnce(async () => {
        const updated = await pending.promise;
        savedModels = savedModels.map((model) =>
          model.internalId === 'account-model' && updated ? updated : model
        );
        return updated;
      });
      await open();
      await select('account-old', 'account-new');
      if (exit === 'presentation') {
        await act(async () => context.setViewMode('super'));
        expect(renderer.root.findAllByType(ModelIdCombobox)).toHaveLength(0);
        await act(async () => context.setViewMode('smart'));
      } else {
        await act(async () => useNavigationStore.getState().setActivePage('models'));
        expect(renderer.root.findAllByType(ModelIdCombobox)).toHaveLength(0);
        await act(async () => useNavigationStore.getState().setActivePage('freeModels'));
      }
      expect(context.savingModelIds.has('account-model')).toBe(true);
      expect(picker('account-old').props.disabled).toBe(true);
      const actions = card('account-old')
        .findAllByType('button')
        .filter((node) => String(node.props['aria-label']).startsWith('btn.'));
      expect(actions).toHaveLength(0);
      await act(async () => {
        picker('account-old').props.onChange('account-new');
        expect(await context.changeSelectedModel('account-model', 'account-new')).toBeUndefined();
      });
      expect(api.updateModel).toHaveBeenCalledExactlyOnceWith('account-model', {
        modelId: 'account-new',
      });
      expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
      expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
      expect(api.getSmartRouterCandidates).toHaveBeenCalledOnce();
      expect(confirm).not.toHaveBeenCalled();
      expect(nexus.setShowAddModelModal).not.toHaveBeenCalled();
      expect(nexus.updateUserModel).not.toHaveBeenCalled();
      const updated = { ...account, modelId: 'account-new' };
      await act(async () => pending.resolve(updated));
      expect(context.savingModelIds.has('account-model')).toBe(false);
      expect(picker('account-new').props.disabled).toBe(false);
      expect([...context.selectedIds]).toEqual(['account-model', 'free-model']);
      expect(nexus.updateUserModel).toHaveBeenCalledExactlyOnceWith(updated, {
        name: updated.name,
        baseUrl: updated.baseUrl,
        anthropicUrl: '',
        apiKey: updated.apiKey,
        modelId: updated.modelId,
        accountSource: updated.accountSource,
      });
      expect(api.updateModel).toHaveBeenCalledOnce();
      expect(showToast).not.toHaveBeenCalled();
    }
  );

  it.each(['rejection', 'missing'] as const)(
    'retains the old model and restores controls after a %s result',
    async (result) => {
      await open();
      if (result === 'rejection')
        vi.mocked(api.updateModel).mockRejectedValueOnce(new Error('Disk write failed'));
      else vi.mocked(api.updateModel).mockResolvedValueOnce(null);
      await select('account-old', 'account-new');
      expect(picker('account-old').props.disabled).toBe(false);
      expect(context.customModels.find((model) => model.id === 'account-model')?.modelId).toBe(
        'account-old'
      );
      expect(nexus.updateUserModel).not.toHaveBeenCalled();
      expect(showToast).toHaveBeenCalledExactlyOnceWith('error', 'error.requestFailed');
      expect([...context.selectedIds]).toEqual(['account-model', 'free-model']);
      expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
      expect(confirm).not.toHaveBeenCalled();
      await select('account-old', 'account-new');
      expect(picker('account-new')).toBeDefined();
      expect(nexus.updateUserModel).toHaveBeenCalledOnce();
    }
  );

  it('serializes a pending model change with priority writes and page reloads', async () => {
    const pending = deferred<ModelConfig | null>();
    vi.mocked(api.updateModel).mockImplementationOnce(async () => {
      const updated = await pending.promise;
      savedModels = savedModels.map((model) =>
        model.internalId === 'account-model' && updated ? updated : model
      );
      return updated;
    });
    await open();
    await select('account-old', 'account-new');
    await act(async () => {
      renderer.root
        .findByType(DndContext)
        .props.onDragEnd({ active: { id: 'free-model' }, over: { id: 'account-model' } });
      useNavigationStore.getState().setActivePage('models');
    });
    await act(async () => useNavigationStore.getState().setActivePage('freeModels'));
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    expect(api.getSmartRouterCandidates).toHaveBeenCalledOnce();
    await act(async () => pending.resolve({ ...account, modelId: 'account-new' }));
    expect(api.setSmartRouterCandidates).toHaveBeenCalledExactlyOnceWith([
      'free-model',
      'account-model',
    ]);
    expect([...context.selectedIds]).toEqual(['free-model', 'account-model']);
    expect(picker('account-new')).toBeDefined();
    expect(api.getSmartRouterCandidates).toHaveBeenCalledTimes(2);
    expect(showToast).not.toHaveBeenCalled();
  });

  it('does not resurrect a candidate removed while its model change was queued', async () => {
    const removal = deferred<Awaited<ReturnType<typeof api.removeSmartRouterCandidate>>>();
    vi.mocked(api.removeSmartRouterCandidate).mockReturnValueOnce(removal.promise);
    await open();
    let removing!: Promise<void>;
    await act(async () => {
      removing = context.removeSelectedModel('account-model');
      picker('account-old').props.onChange('account-new');
    });
    expect(api.updateModel).not.toHaveBeenCalled();
    savedOrder = ['free-model'];
    await act(async () => {
      removal.resolve(config());
      await removing;
    });
    expect([...context.selectedIds]).toEqual(['free-model']);
    expect(context.customModels.some((model) => model.id === 'account-model')).toBe(false);
    expect(api.updateModel).not.toHaveBeenCalled();
    expect(nexus.updateUserModel).not.toHaveBeenCalled();
    expect(showToast).toHaveBeenCalledExactlyOnceWith('error', 'error.requestFailed');
  });

  it('queues a drag removal behind a pending model selection and retains the saved model', async () => {
    const pending = deferred<ModelConfig | null>();
    vi.mocked(api.updateModel).mockImplementationOnce(async () => {
      const updated = await pending.promise;
      savedModels = savedModels.map((model) =>
        model.internalId === 'account-model' && updated ? updated : model
      );
      return updated;
    });
    await open();
    await select('account-old', 'account-new');
    expect(picker('account-old').props.disabled).toBe(true);
    await act(async () => {
      renderer.root.findByType(DndContext).props.onDragStart({ active: { id: 'account-model' } });
    });
    await act(async () => {
      renderer.root.findByType(DndContext).props.onDragEnd({
        active: { id: 'account-model' },
        over: { id: 'router-remove-drop-zone' },
      });
    });
    expect(api.removeSmartRouterCandidate).not.toHaveBeenCalled();
    expect([...context.selectedIds]).toEqual(['account-model', 'free-model']);
    await act(async () => pending.resolve({ ...account, modelId: 'account-new' }));
    expect(api.removeSmartRouterCandidate).toHaveBeenCalledExactlyOnceWith('account-model');
    expect([...context.selectedIds]).toEqual(['free-model']);
    expect(context.customModels.some((model) => model.id === 'account-model')).toBe(false);
    expect(savedModels.find((model) => model.internalId === 'account-model')?.modelId).toBe(
      'account-new'
    );
    expect(api.deleteModel).not.toHaveBeenCalled();
    expect(api.setSmartRouterCandidates).not.toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();
    expect(showToast).not.toHaveBeenCalled();
    expect(nexus.setShowAddModelModal).not.toHaveBeenCalled();
  });
});
