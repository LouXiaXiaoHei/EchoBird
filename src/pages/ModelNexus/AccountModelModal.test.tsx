import { useLayoutEffect } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import * as api from '../../api/tauri';
import * as accountApi from '../../api/accountProxy';
import { AddModelModal, ModelNexusProvider } from './ModelNexus';
import { useModelNexus } from './context';

const { showToast, addSelectedModel } = vi.hoisted(() => ({
  showToast: vi.fn(),
  addSelectedModel: vi.fn(),
}));
vi.mock('../../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock('../../components/Toast', () => ({ useToast: () => ({ showToast }) }));
vi.mock('../../components/ModelIdCombobox', () => ({ ModelIdCombobox: () => null }));
vi.mock('../../components', () => ({
  ModelCard: () => null,
  ModelCardSkeleton: () => null,
  ModelIdCombobox: () => null,
  getModelIcon: () => null,
}));
vi.mock('../FreeModels/FreeModels', () => ({
  useFreeModels: () => ({ addSelectedModel, updateSelectedModel: vi.fn(), selectedIds: new Set() }),
}));
vi.mock('../../api/tauri', () => ({
  getModels: vi.fn().mockResolvedValue([]),
  addModel: vi.fn(),
  updateModel: vi.fn(),
  deleteModel: vi.fn().mockResolvedValue(undefined),
  queryModelUsage: vi.fn(),
  SMART_ROUTER_CANDIDATE_LIMIT: 10,
}));
vi.mock('../../api/accountProxy', async (original) => ({
  ...(await original<typeof import('../../api/accountProxy')>()),
  listProxyAccounts: vi
    .fn()
    .mockResolvedValue([{ source: { provider: 'codex', accountId: 'saved' }, label: 'Saved' }]),
  getAccountProxyModels: vi.fn().mockResolvedValue(['gpt-fixture', 'gpt-other']),
}));

let renderer: ReactTestRenderer;
let context: ReturnType<typeof useModelNexus>;
function Harness() {
  const state = useModelNexus();
  useLayoutEffect(() => {
    context = state;
  });
  return <AddModelModal />;
}
const saved = {
  internalId: 'proxy-fixture',
  name: 'Codex',
  baseUrl: 'account://echobird',
  apiKey: '',
  modelId: 'gpt-fixture',
  scope: 'smartRouter' as const,
  accountSource: { provider: 'codex' as const, accountId: 'saved' },
};
async function open(accountId = 'saved') {
  await act(async () => {
    renderer = create(
      <ModelNexusProvider>
        <Harness />
      </ModelNexusProvider>
    );
  });
  await act(async () => {
    context.setNewModelForm({
      name: 'Codex',
      baseUrl: 'account://echobird',
      anthropicUrl: '',
      apiKey: 'discard-this-key',
      modelId: 'gpt-fixture',
      accountSource: { provider: 'codex', accountId },
    });
    context.setModelModalDestination('freeRouter');
    context.setShowAddModelModal(true);
  });
}
function saveButton() {
  return renderer.root
    .findAllByType('button')
    .find((button) => button.children.includes('model.enterSave'))!;
}
beforeEach(() => {
  const stored = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: vi.fn((key: string, value: string) => stored.set(key, value)),
  });
  vi.mocked(accountApi.getAccountProxyModels)
    .mockReset()
    .mockResolvedValue(['gpt-fixture', 'gpt-other']);
  vi.mocked(api.addModel).mockResolvedValue(saved);
  addSelectedModel.mockResolvedValue(undefined);
});
afterEach(() => {
  act(() => renderer?.unmount());
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

it('hides the key and saves only the binding through the existing router workflow', async () => {
  await open();
  expect(
    renderer.root.findAllByType('label').some((label) => label.children.includes('model.apiKey'))
  ).toBe(false);
  expect(
    renderer.root.findAllByType('input').some((input) => input.props.placeholder === 'sk-...')
  ).toBe(false);
  expect(api.addModel).not.toHaveBeenCalled();
  expect(api.queryModelUsage).not.toHaveBeenCalled();
  expect(accountApi.getAccountProxyModels).toHaveBeenCalledExactlyOnceWith(saved.accountSource);
  await act(async () => {
    await saveButton().props.onClick();
  });
  expect(api.addModel).toHaveBeenCalledWith(
    expect.objectContaining({
      apiKey: '',
      accountSource: saved.accountSource,
      scope: 'smartRouter',
    })
  );
  expect(addSelectedModel).toHaveBeenCalledWith(
    expect.objectContaining({ internalId: saved.internalId, modelId: saved.modelId })
  );
  expect(context.showAddModelModal).toBe(false);
  vi.mocked(accountApi.getAccountProxyModels).mockClear();
  await act(async () => {
    await context.handleCardEdit(saved);
  });
  expect(context.newModelForm.modelIdOptions).toEqual(['gpt-fixture', 'gpt-other']);
  expect(context.newModelForm.modelId).toBe(saved.modelId);
  expect(accountApi.getAccountProxyModels).not.toHaveBeenCalled();
  expect(renderer.root.findByProps({ id: 'model-config-dialog-title' }).children).toEqual([
    '编辑账号配置',
  ]);
  await act(async () => {
    await context.handleCardEdit({ ...saved, accountSource: undefined });
  });
  expect(renderer.root.findByProps({ id: 'model-config-dialog-title' }).children).toEqual([
    'model.editConfig',
  ]);
});

it('requires a saved account before writing and keeps the form available', async () => {
  await open('');
  await act(async () => {
    await saveButton().props.onClick();
  });
  expect(api.addModel).not.toHaveBeenCalled();
  expect(showToast).toHaveBeenCalledWith('warning', 'accountProxy.selectRequired');
  expect(context.showAddModelModal).toBe(true);
});

it('removes the new model if adding the router candidate fails', async () => {
  await open();
  addSelectedModel.mockRejectedValueOnce(new Error('Fixture router failure'));
  await act(async () => {
    await saveButton().props.onClick();
  });
  expect(api.deleteModel).toHaveBeenCalledWith(saved.internalId);
  expect(showToast).toHaveBeenCalledWith('error', 'error.requestFailed');
  expect(context.showAddModelModal).toBe(true);
  expect(context.newModelForm.accountSource).toEqual(saved.accountSource);
});

it.each(['success', 'failure'])(
  'ignores a late directory %s during the close animation',
  async (outcome) => {
    vi.useFakeTimers();
    let resolve!: (models: string[]) => void;
    let reject!: (error: Error) => void;
    vi.mocked(accountApi.getAccountProxyModels).mockReturnValueOnce(
      new Promise((done, fail) => {
        resolve = done;
        reject = fail;
      })
    );
    await open();
    expect(accountApi.getAccountProxyModels).toHaveBeenCalledExactlyOnceWith(saved.accountSource);
    act(() => context.closeModelModal());
    expect(context.modelModalAnimatingOut).toBe(true);
    expect(context.showAddModelModal).toBe(true);
    const closingForm = context.newModelForm;
    await act(async () => {
      if (outcome === 'success') resolve(['late-model']);
      else reject(new Error('Late directory error'));
    });
    expect(context.newModelForm).toBe(closingForm);
    expect(localStorage.setItem).not.toHaveBeenCalled();
    expect(showToast).not.toHaveBeenCalled();
    expect(api.addModel).not.toHaveBeenCalled();
  }
);
