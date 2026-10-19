import { useLayoutEffect } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import * as api from '../../api/tauri';
import { ModelCard } from '../../components/cards/ModelCard';
import { ModelNexusProvider } from './provider';
import { useModelNexus } from './context';

vi.mock('../../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock('../../components/Toast', () => ({ useToast: () => ({ showToast: vi.fn() }) }));
vi.mock('../../components/ConfirmDialog', () => ({ useConfirm: () => vi.fn() }));
vi.mock('../../components/LatencyIndicator', () => ({ LatencyIndicator: () => null }));
vi.mock('../../api/tauri', () => ({
  getModels: vi.fn(),
  pingModel: vi.fn(),
  finishConnectionTestBatch: vi.fn().mockResolvedValue(undefined),
  testModel: vi.fn(),
  queryModelUsage: vi.fn(),
  updateModel: vi.fn(),
}));

let renderer: ReactTestRenderer | undefined;
let context: ReturnType<typeof useModelNexus>;
function Harness() {
  const state = useModelNexus();
  useLayoutEffect(() => {
    context = state;
  });
  return (
    <ModelCard id="fixture" name="Fixture" type="cloud" latency={state.modelLatencies.fixture} />
  );
}
beforeEach(() => {
  vi.mocked(api.getModels).mockResolvedValue([
    { internalId: 'fixture', name: 'Fixture', baseUrl: 'https://fixture.test/v1', apiKey: '' },
  ]);
});
afterEach(() => {
  act(() => renderer?.unmount());
  vi.clearAllMocks();
});

it.each([
  [{ success: true, latency: 123 }, '123ms', 'text-green-500'],
  [{ success: true, latency: 1000 }, '1000ms', 'text-red-500'],
  [{ success: true, latency: 1001 }, 'model.timedOut', 'text-red-500'],
  [{ success: false, latency: -1, error: 'timeout' }, 'model.timedOut', 'text-red-500'],
  [{ success: false, latency: -1, error: 'connection refused' }, 'Error', 'text-red-500'],
])('renders connection results distinctly: %j', async (result, label, color) => {
  vi.mocked(api.pingModel).mockResolvedValue({ url: 'https://fixture.test', ...result });
  await act(async () => {
    renderer = create(
      <ModelNexusProvider>
        <Harness />
      </ModelNexusProvider>
    );
  });
  expect(api.pingModel).not.toHaveBeenCalled();
  await act(async () => context.pingAllModels());
  const value = renderer!.root
    .findAllByType('span')
    .find((span) => span.children.join('') === label);
  expect(value).toBeDefined();
  expect(value!.props.className).toContain(color);
  expect(api.pingModel).toHaveBeenCalledExactlyOnceWith('fixture', expect.any(String));
  expect(api.finishConnectionTestBatch).toHaveBeenCalledExactlyOnceWith(
    vi.mocked(api.pingModel).mock.calls[0][1]
  );
  expect(api.testModel).not.toHaveBeenCalled();
  expect(api.queryModelUsage).not.toHaveBeenCalled();
  expect(api.updateModel).not.toHaveBeenCalled();
});

it('uses one round for all rows and starts a fresh round on the next click', async () => {
  vi.mocked(api.getModels).mockResolvedValue([
    { internalId: 'fixture', name: 'Fixture', baseUrl: 'https://fixture.test/v1', apiKey: '' },
    { internalId: 'second', name: 'Second', baseUrl: 'https://fixture.test/anthropic', apiKey: '' },
  ]);
  vi.mocked(api.pingModel).mockResolvedValue({
    url: 'https://fixture.test',
    success: true,
    latency: 220,
  });
  await act(async () => {
    renderer = create(
      <ModelNexusProvider>
        <Harness />
      </ModelNexusProvider>
    );
  });
  expect(api.pingModel).not.toHaveBeenCalled();
  expect(api.finishConnectionTestBatch).not.toHaveBeenCalled();
  await act(async () => context.pingAllModels());
  const batch = vi.mocked(api.pingModel).mock.calls[0][1];
  expect(batch).toEqual(expect.any(String));
  expect(api.pingModel).toHaveBeenNthCalledWith(2, 'second', batch);
  expect(api.finishConnectionTestBatch).toHaveBeenCalledExactlyOnceWith(batch);
  expect(context.modelLatencies).toEqual({ fixture: 220, second: 220 });
  expect(context.pingingModelIds.size).toBe(0);
  expect(context.isTesting).toBe(false);
  await act(async () => context.pingAllModels());
  expect(api.pingModel).toHaveBeenCalledTimes(4);
  expect(vi.mocked(api.pingModel).mock.calls[2][1]).not.toBe(batch);
  expect(api.finishConnectionTestBatch).toHaveBeenCalledTimes(2);
  expect(api.testModel).not.toHaveBeenCalled();
  expect(api.queryModelUsage).not.toHaveBeenCalled();
  expect(api.updateModel).not.toHaveBeenCalled();
});
