import { useLayoutEffect, useState } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { AccountModelFields } from './AccountModelFields';
import type { NewModelForm } from './context';
import * as api from '../../api/accountProxy';
import { ModelIdCombobox } from '../../components/ModelIdCombobox';

const { showToast } = vi.hoisted(() => ({ showToast: vi.fn() }));
vi.mock('../../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock('../../components/Toast', () => ({ useToast: () => ({ showToast }) }));
vi.mock('../../components/ModelIdCombobox', () => ({ ModelIdCombobox: () => null }));
vi.mock('../../api/accountProxy', async (original) => ({
  ...(await original<typeof import('../../api/accountProxy')>()),
  listProxyAccounts: vi.fn(),
  getAccountProxyModels: vi.fn(),
}));

let renderer: ReactTestRenderer;
let current: NewModelForm;
let stored: Map<string, string>;
let setItem: ReturnType<typeof vi.fn>;
function Harness({ accountId = '', modelId = '' }: { accountId?: string; modelId?: string }) {
  const [form, setForm] = useState<NewModelForm>({
    name: 'Codex',
    baseUrl: '',
    anthropicUrl: '',
    apiKey: '',
    modelId,
    accountSource: { provider: 'codex', accountId },
  });
  useLayoutEffect(() => {
    current = form;
  });
  return (
    <AccountModelFields
      source={form.accountSource!}
      modelId={form.modelId}
      modelIdOptions={form.modelIdOptions}
      setForm={setForm}
    />
  );
}
async function open(accountId = '', modelId = '') {
  await act(async () => {
    renderer = create(<Harness accountId={accountId} modelId={modelId} />);
  });
}
function field(label: string) {
  return renderer.root
    .findAllByType(ModelIdCombobox)
    .find((node) => node.props.ariaLabel === label)!;
}
beforeEach(() => {
  stored = new Map();
  setItem = vi.fn((key: string, value: string) => stored.set(key, value));
  vi.stubGlobal('localStorage', { getItem: (key: string) => stored.get(key) ?? null, setItem });
  vi.mocked(api.listProxyAccounts)
    .mockReset()
    .mockResolvedValue([
      { source: { provider: 'codex', accountId: 'first' }, label: 'First' },
      { source: { provider: 'codex', accountId: 'second' }, label: 'Second' },
    ]);
  vi.mocked(api.getAccountProxyModels).mockReset().mockResolvedValue(['gpt-fixture', 'gpt-other']);
});
afterEach(() => {
  act(() => renderer?.unmount());
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

it('loads only saved accounts before selection, then automatically reads and saves the directory', async () => {
  await open();
  expect(api.listProxyAccounts).toHaveBeenCalledWith('codex');
  expect(api.getAccountProxyModels).not.toHaveBeenCalled();
  expect(setItem).not.toHaveBeenCalled();
  await act(async () => {
    field('accountProxy.account').props.onChange('first');
  });
  expect(api.getAccountProxyModels).toHaveBeenCalledExactlyOnceWith({
    provider: 'codex',
    accountId: 'first',
  });
  expect(current.modelIdOptions).toEqual(['gpt-fixture', 'gpt-other']);
  expect(current.modelId).toBe('gpt-fixture');
  expect(current.apiKey).toBe('');
  expect(api.getCachedAccountProxyModels(current.accountSource!)).toEqual(current.modelIdOptions);
  expect(setItem).toHaveBeenCalledOnce();
  expect(renderer.root.findAllByType('button')).toHaveLength(0);
  expect(field('model.modelId').props).toMatchObject({
    selectOnly: true,
    refreshLabel: 'btn.refresh',
  });
});

it('reuses the persisted account directory on a new edit form and preserves the selected model', async () => {
  await open('first');
  act(() => renderer.unmount());
  vi.mocked(api.getAccountProxyModels).mockClear();
  setItem.mockClear();
  await open('first', 'gpt-other');
  expect(current.modelIdOptions).toEqual(['gpt-fixture', 'gpt-other']);
  expect(current.modelId).toBe('gpt-other');
  expect(api.getAccountProxyModels).not.toHaveBeenCalled();
  expect(setItem).not.toHaveBeenCalled();
});

it('shows the account label after loading without displaying the internal account ID while pending', async () => {
  let resolve!: (accounts: api.ProxyAccount[]) => void;
  vi.mocked(api.listProxyAccounts).mockReturnValueOnce(
    new Promise((done) => {
      resolve = done;
    })
  );
  await open('first');
  expect(field('accountProxy.account').props).toMatchObject({
    disabled: true,
    value: '',
    placeholder: '—',
  });
  await act(async () =>
    resolve([
      { source: { provider: 'codex', accountId: 'first' }, label: 'First', plan: 'prolite' },
    ])
  );
  expect(field('accountProxy.account').props).toMatchObject({
    disabled: false,
    value: 'first',
    optionLabels: { first: 'First · Pro 100' },
  });
});

it('isolates account and provider caches and restores them without another request', async () => {
  await open('first');
  vi.mocked(api.getAccountProxyModels).mockResolvedValueOnce(['second-model']);
  await act(async () => field('accountProxy.account').props.onChange('second'));
  expect(current.modelId).toBe('second-model');
  await act(async () => field('accountProxy.account').props.onChange('first'));
  expect(current.modelIdOptions).toEqual(['gpt-fixture', 'gpt-other']);
  await act(async () => field('accountProxy.provider').props.onChange('kilo'));
  expect(current.modelIdOptions).toEqual([]);
  expect(current.accountSource).toEqual({ provider: 'kilo', accountId: '' });
  expect(api.getAccountProxyModels).toHaveBeenCalledTimes(2);
  vi.mocked(api.getAccountProxyModels).mockResolvedValueOnce(['kilo-model']);
  await act(async () => field('accountProxy.account').props.onChange('first'));
  expect(current.modelIdOptions).toEqual(['kilo-model']);
  expect(api.getCachedAccountProxyModels({ provider: 'codex', accountId: 'first' })).toEqual([
    'gpt-fixture',
    'gpt-other',
  ]);
  expect(api.getCachedAccountProxyModels({ provider: 'kilo', accountId: 'first' })).toEqual([
    'kilo-model',
  ]);
});

it('refreshes explicitly, preserves a valid choice, and retains the snapshot when refresh fails', async () => {
  await open('first');
  act(() => field('model.modelId').props.onChange('gpt-other'));
  vi.mocked(api.getAccountProxyModels).mockResolvedValueOnce(['gpt-other', 'new-model']);
  await act(async () => field('model.modelId').props.onRefresh());
  expect(current.modelId).toBe('gpt-other');
  expect(current.modelIdOptions).toEqual(['gpt-other', 'new-model']);
  vi.mocked(api.getAccountProxyModels).mockRejectedValueOnce(new Error('Directory unavailable'));
  setItem.mockClear();
  await act(async () => field('model.modelId').props.onRefresh());
  expect(current.modelId).toBe('gpt-other');
  expect(current.modelIdOptions).toEqual(['gpt-other', 'new-model']);
  expect(api.getCachedAccountProxyModels(current.accountSource!)).toEqual([
    'gpt-other',
    'new-model',
  ]);
  expect(setItem).not.toHaveBeenCalled();
  expect(showToast).toHaveBeenCalledWith('error', 'Error: Directory unavailable');
});

it('ignores old responses when changing accounts or exiting and prevents duplicate refreshes', async () => {
  let resolve!: (models: string[]) => void;
  vi.mocked(api.getAccountProxyModels).mockReturnValueOnce(
    new Promise((done) => {
      resolve = done;
    })
  );
  await open('first');
  expect(field('model.modelId').props.disabled).toBe(true);
  act(() => {
    field('model.modelId').props.onRefresh();
    field('model.modelId').props.onRefresh();
  });
  expect(api.getAccountProxyModels).toHaveBeenCalledOnce();
  await act(async () => field('accountProxy.account').props.onChange('second'));
  await act(async () => resolve(['old-account-model']));
  expect(current.accountSource?.accountId).toBe('second');
  expect(current.modelIdOptions).toEqual(['gpt-fixture', 'gpt-other']);
  expect(
    api.getCachedAccountProxyModels({ provider: 'codex', accountId: 'first' })
  ).toBeUndefined();
  vi.mocked(api.getAccountProxyModels).mockReturnValueOnce(
    new Promise((done) => {
      resolve = done;
    })
  );
  act(() => {
    field('model.modelId').props.onRefresh();
    renderer.unmount();
  });
  setItem.mockClear();
  await act(async () => resolve(['late-model']));
  expect(setItem).not.toHaveBeenCalled();
  expect(showToast).not.toHaveBeenCalled();
});

it('caches a successful empty directory and retries it only on refresh', async () => {
  vi.mocked(api.getAccountProxyModels).mockResolvedValueOnce([]);
  await open('first');
  expect(current.modelId).toBe('');
  expect(current.modelIdOptions).toEqual([]);
  act(() => renderer.unmount());
  vi.mocked(api.getAccountProxyModels).mockClear();
  await open('first');
  expect(api.getAccountProxyModels).not.toHaveBeenCalled();
  await act(async () => field('model.modelId').props.onRefresh());
  expect(current.modelId).toBe('gpt-fixture');
});

it('recovers a damaged cache without persisting account credentials', async () => {
  const source = { provider: 'codex' as const, accountId: 'first' };
  api.cacheAccountProxyModels(source, ['old-model']);
  const key = [...stored.keys()][0];
  stored.set(key, '{invalid');
  expect(api.getCachedAccountProxyModels(source)).toBeUndefined();
  await open('first');
  expect(current.modelIdOptions).toEqual(['gpt-fixture', 'gpt-other']);
  expect([...stored.values()]).toEqual([JSON.stringify(['gpt-fixture', 'gpt-other'])]);
});
