import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import {
  cancelSmartRouterProbe,
  testSmartRouterCandidate,
  type SmartRouterProbeResult,
} from '../../api/smartRouter';
import type { RouteModelNode } from './provider';
import { finishConnectionTestBatch } from '../../api/models';

function binding(model: RouteModelNode) {
  return JSON.stringify([
    model.modelId,
    model.baseUrl,
    model.anthropicUrl,
    model.apiKey,
    model.accountSource,
  ]);
}

export function useRouteResponseTests(models: RouteModelNode[], active: boolean) {
  const [pendingIds, setPendingIds] = useState<Set<string>>(new Set());
  const [savedResults, setSavedResults] = useState<
    Record<string, { binding: string; result: SmartRouterProbeResult }>
  >({});
  const generation = useRef(0);
  const running = useRef(false);
  const requests = useRef(new Map<string, string>());
  const batchId = useRef<string | null>(null);
  const bindings = new Map(models.map((model) => [model.id, binding(model)]));
  const bindingsKey = JSON.stringify([...bindings].sort(([a], [b]) => a.localeCompare(b)));

  const cancel = useCallback(() => {
    generation.current++;
    running.current = false;
    for (const requestId of requests.current.values()) {
      void cancelSmartRouterProbe(requestId).catch(() => undefined);
    }
    requests.current.clear();
    if (batchId.current) {
      void finishConnectionTestBatch(batchId.current).catch(() => undefined);
      batchId.current = null;
    }
    setPendingIds(new Set());
  }, []);

  useLayoutEffect(() => cancel, [active, bindingsKey, cancel]);

  const test = async () => {
    if (!active || running.current || models.length === 0) return;
    running.current = true;
    const attempt = ++generation.current;
    const batch = crypto.randomUUID();
    batchId.current = batch;
    const queue = models.map((model) => ({ model, binding: binding(model) }));
    let next = 0;
    setPendingIds(new Set(models.map((model) => model.id)));
    const worker = async () => {
      while (generation.current === attempt && next < queue.length) {
        const entry = queue[next++];
        const { model } = entry;
        const requestId = crypto.randomUUID();
        requests.current.set(model.id, requestId);
        let result: SmartRouterProbeResult;
        try {
          result = await testSmartRouterCandidate(
            model.internalId,
            model.modelId,
            requestId,
            batch
          );
        } catch {
          result = { latencyMs: null, error: 'request_failed' };
        }
        if (generation.current !== attempt) return;
        requests.current.delete(model.id);
        setSavedResults((previous) => ({
          ...previous,
          [model.id]: { binding: entry.binding, result },
        }));
        setPendingIds((previous) => {
          const remaining = new Set(previous);
          remaining.delete(model.id);
          return remaining;
        });
      }
    };
    try {
      await Promise.all([worker(), worker()]);
    } finally {
      if (batchId.current === batch) {
        batchId.current = null;
        void finishConnectionTestBatch(batch).catch(() => undefined);
      }
      if (generation.current === attempt) running.current = false;
    }
  };

  const results: Record<string, SmartRouterProbeResult> = {};
  for (const [id, value] of Object.entries(savedResults)) {
    if (bindings.get(id) === value.binding) results[id] = value.result;
  }
  return { pendingIds, results, isTesting: pendingIds.size > 0, test };
}
