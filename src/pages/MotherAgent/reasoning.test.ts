import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ModelConfig } from '../../api/types';
import { getReasoningLevels, loadReasoningPreferences } from './reasoning';

const model = (modelId: string, changes: Partial<ModelConfig> = {}): ModelConfig => ({
  internalId: 'fixture',
  name: modelId,
  modelId,
  baseUrl: 'https://api.example.test/v1',
  apiKey: 'fixture',
  ...changes,
});

afterEach(() => vi.unstubAllGlobals());

describe('Mother reasoning levels', () => {
  it.each(['gpt-5', 'gpt-5.2-pro', 'claude-opus-4-5', 'deepseek-flash', 'custom-model'])(
    'offers the same five choices for %s through either protocol',
    (id) => {
      for (const changes of [
        {},
        { baseUrl: '', anthropicUrl: 'https://api.example.test/anthropic' },
        { anthropicUrl: 'https://api.example.test/anthropic' },
      ]) {
        expect(getReasoningLevels(model(id, changes))).toEqual([
          'low',
          'medium',
          'high',
          'xhigh',
          'max',
        ]);
      }
    }
  );

  it('keeps all five choices for routers, gateways and incomplete endpoints', () => {
    for (const changes of [
      { scope: 'smartRouter' as const },
      { baseUrl: 'https://openrouter.ai/api/v1' },
      { baseUrl: 'http://127.0.0.1:55536' },
      { baseUrl: '' },
    ]) {
      expect(getReasoningLevels(model('custom-model', changes))).toEqual([
        'low',
        'medium',
        'high',
        'xhigh',
        'max',
      ]);
    }
    expect(getReasoningLevels()).toEqual([]);
  });

  it('recovers safely from missing, malformed, and stale local preferences', () => {
    for (const stored of [null, '{bad', 'null', '[]', '"high"']) {
      vi.stubGlobal('localStorage', { getItem: () => stored });
      expect(loadReasoningPreferences()).toEqual({});
    }
    vi.stubGlobal('localStorage', {
      getItem: () => '{"model":"high","other":"invalid","object":["high"]}',
    });
    expect(loadReasoningPreferences()).toEqual({ model: 'high' });
    vi.stubGlobal('localStorage', {
      getItem: () => {
        throw new Error('unavailable');
      },
    });
    expect(loadReasoningPreferences()).toEqual({});
  });
});
