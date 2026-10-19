import { describe, expect, it } from 'vitest';
import catalog from '../../docs/api/free-models/index.json';
import bundledDirectory from './modelDirectory.json';
import remoteDirectory from '../../docs/api/model-directory/index.json';

describe('free model catalog', () => {
  it('contains unique, directly configurable model entries', () => {
    expect(catalog.models.length).toBeGreaterThan(20);
    expect(new Set(catalog.models.map((model) => model.id)).size).toBe(catalog.models.length);

    for (const model of catalog.models) {
      expect(model.modelId).not.toBe('');
      expect(model.baseUrl.startsWith('https://')).toBe(true);
      expect(model.baseUrl).not.toContain('{');
      expect(model.verifiedAt).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    }
  });

  it('does not expose source metadata in the public directory', () => {
    expect(catalog).not.toHaveProperty('source');
  });

  it('keeps Agnes and ModelScope endpoints aligned in both directories', () => {
    for (const [providerId, name, baseUrl, anthropicUrl, websiteUrl] of [
      [
        'agnes',
        'Agnes AI',
        'https://apihub.agnes-ai.com/v1',
        'https://apihub.agnes-ai.com',
        'https://platform.agnes-ai.com/',
      ],
      [
        'modelscope',
        'ModelScope',
        'https://api-inference.modelscope.cn/v1',
        'https://api-inference.modelscope.cn',
        'https://modelscope.cn/',
      ],
    ]) {
      const models = catalog.models.filter((model) => model.providerId === providerId);
      expect(models.length).toBeGreaterThan(0);
      for (const model of models) {
        expect(model).toMatchObject({ provider: name, baseUrl, anthropicUrl, websiteUrl });
      }
      expect(remoteDirectory.providers.find((entry) => entry.name === name)).toMatchObject({
        baseUrl,
        anthropicUrl,
        url: websiteUrl,
      });
      expect(bundledDirectory.providers.find((entry) => entry.name === name)).toEqual(
        remoteDirectory.providers.find((entry) => entry.name === name)
      );
    }
    expect(
      catalog.models.filter((model) => model.providerId === 'agnes').map((model) => model.modelId)
    ).toEqual(['agnes-2.5-flash']);
  });
});
