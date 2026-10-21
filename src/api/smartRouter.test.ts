import { invoke } from '@tauri-apps/api/core';
import { afterEach, expect, it, vi } from 'vitest';
import { testSmartRouterCandidate } from './smartRouter';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
afterEach(() => vi.resetAllMocks());

it('passes the selected router to the native probe without changing candidate or batch identity', async () => {
  vi.mocked(invoke).mockResolvedValue({ latencyMs: 23, error: null });
  expect(
    await testSmartRouterCandidate('keyless', 'free-model', 'request', 'batch', 'selected-router')
  ).toEqual({
    latencyMs: 23,
    error: null,
  });
  expect(invoke).toHaveBeenCalledExactlyOnceWith('test_smart_router_candidate', {
    internalId: 'keyless',
    modelId: 'free-model',
    requestId: 'request',
    batchId: 'batch',
    routerId: 'selected-router',
  });
});
