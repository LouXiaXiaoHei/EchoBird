import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from '../../api/smartRouter';
import type { RouteModelNode } from './provider';
import { useRouteResponseTests } from './useRouteResponseTests';

vi.mock('../../api/smartRouter', () => ({
  testSmartRouterCandidate: vi.fn(),
  cancelSmartRouterProbe: vi.fn().mockResolvedValue(undefined),
}));

describe('explicit Smart Router response tests', () => {
  let renderer: ReactTestRenderer;
  let hook: ReturnType<typeof useRouteResponseTests>;
  let models: RouteModelNode[];
  let active: boolean;
  let replies: ((result: api.SmartRouterProbeResult) => void)[];
  function Harness() {
    hook = useRouteResponseTests(models, active);
    return null;
  }
  async function render() {
    await act(async () => {
      if (renderer) renderer.update(<Harness />);
      else renderer = create(<Harness />);
    });
  }
  async function reply(index: number, result: api.SmartRouterProbeResult) {
    await act(async () => replies[index](result));
  }
  beforeEach(() => {
    renderer = undefined as unknown as ReactTestRenderer;
    active = true;
    models = ['api', 'account', 'keyless'].map((id) => ({
      id,
      internalId: id,
      provider: id,
      modelId: `${id}-model`,
      baseUrl: `https://${id}.example/v1`,
      ...(id === 'account'
        ? { accountSource: { provider: 'codex' as const, accountId: 'fixture-account' } }
        : {}),
    }));
    replies = [];
    vi.mocked(api.testSmartRouterCandidate).mockImplementation(
      () => new Promise((resolve) => replies.push(resolve))
    );
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
  });
  afterEach(async () => {
    if (renderer) await act(async () => renderer.unmount());
    vi.clearAllMocks();
    vi.restoreAllMocks();
  });

  it('does nothing on navigation; tests every selected binding with at most two requests', async () => {
    active = false;
    await render();
    await act(async () => hook.test());
    active = true;
    await render();
    expect(api.testSmartRouterCandidate).not.toHaveBeenCalled();
    await act(async () => {
      void hook.test();
    });
    expect(api.testSmartRouterCandidate).toHaveBeenCalledTimes(2);
    expect(hook.pendingIds.size).toBe(3);
    await act(async () => hook.test());
    expect(api.testSmartRouterCandidate).toHaveBeenCalledTimes(2);
    await reply(0, { latencyMs: 0, error: null });
    expect(api.testSmartRouterCandidate).toHaveBeenCalledTimes(3);
    expect(api.testSmartRouterCandidate).toHaveBeenNthCalledWith(
      2,
      'account',
      'account-model',
      expect.any(String)
    );
    await reply(1, { latencyMs: null, error: 'HTTP 401' });
    await reply(2, { latencyMs: null, error: 'timeout' });
    expect(hook.isTesting).toBe(false);
    expect(hook.results.api.latencyMs).toBe(0);
    expect(hook.results.account.error).toBe('HTTP 401');
    expect(hook.results.keyless.error).toBe('timeout');
    expect(api.cancelSmartRouterProbe).not.toHaveBeenCalled();
    await act(async () => {
      void hook.test();
    });
    expect(api.testSmartRouterCandidate).toHaveBeenCalledTimes(5);
    expect(
      new Set(vi.mocked(api.testSmartRouterCandidate).mock.calls.map((call) => call[2])).size
    ).toBe(5);
  });

  it('cancels on leaving the list, stops queued requests and ignores late results', async () => {
    await render();
    await act(async () => {
      void hook.test();
    });
    const ids = vi.mocked(api.testSmartRouterCandidate).mock.calls.map((call) => call[2]);
    active = false;
    await render();
    expect(vi.mocked(api.cancelSmartRouterProbe).mock.calls.map((call) => call[0])).toEqual(ids);
    await reply(0, { latencyMs: 100, error: null });
    await reply(1, { latencyMs: 200, error: null });
    expect(api.testSmartRouterCandidate).toHaveBeenCalledTimes(2);
    expect(hook.results).toEqual({});
    expect(hook.isTesting).toBe(false);
    active = true;
    await render();
    expect(api.testSmartRouterCandidate).toHaveBeenCalledTimes(2);
  });

  it.each(['model', 'account', 'endpoint', 'remove'])(
    'cancels when the %s binding changes',
    async (change) => {
      await render();
      await act(async () => {
        void hook.test();
      });
      models = models.map((model) => ({ ...model }));
      if (change === 'model') models[1].modelId = 'new-model';
      if (change === 'account')
        models[1].accountSource = { provider: 'codex', accountId: 'new-account' };
      if (change === 'endpoint') models[0].baseUrl = 'https://changed.example/v1';
      if (change === 'remove') models = models.slice(1);
      await render();
      expect(api.cancelSmartRouterProbe).toHaveBeenCalledTimes(2);
      await reply(0, { latencyMs: 10, error: null });
      await reply(1, { latencyMs: 10, error: null });
      expect(hook.results).toEqual({});
      expect(api.testSmartRouterCandidate).toHaveBeenCalledTimes(2);
    }
  );

  it('preserves results and in-flight requests when only priority or pause changes', async () => {
    await render();
    await act(async () => {
      void hook.test();
    });
    await reply(0, { latencyMs: 321, error: null });
    models = [models[2], models[0], models[1]].map((model) => ({ ...model }));
    await render();
    expect(api.cancelSmartRouterProbe).not.toHaveBeenCalled();
    expect(hook.results.api.latencyMs).toBe(321);
    await reply(1, { latencyMs: 456, error: null });
    await reply(2, { latencyMs: 789, error: null });
    expect(hook.results.account.latencyMs).toBe(456);
    active = false;
    await render();
    expect(hook.results.api.latencyMs).toBe(321);
    models = models.map((model) => (model.id === 'api' ? { ...model, modelId: 'other' } : model));
    await render();
    expect(hook.results.api).toBeUndefined();
  });

  it('reports IPC failures and cancels owned requests on unmount', async () => {
    vi.mocked(api.testSmartRouterCandidate).mockRejectedValueOnce(new Error('IPC unavailable'));
    await render();
    await act(async () => {
      void hook.test();
    });
    expect(hook.results.api.error).toBe('request_failed');
    expect(api.testSmartRouterCandidate).toHaveBeenCalledTimes(3);
    await act(async () => renderer.unmount());
    expect(api.cancelSmartRouterProbe).toHaveBeenCalledTimes(2);
    renderer = undefined as unknown as ReactTestRenderer;
  });
});
