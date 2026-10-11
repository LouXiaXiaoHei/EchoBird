import { describe, expect, it } from 'vitest';
import bundled from './localModelStore.json';
import remote from '../../docs/api/store/models.json';
import { normalizeStoreModels } from '../api/types';

describe('local model store catalog', () => {
  it('ships the same downloadable catalog remotely and in the desktop bundle', () => {
    expect(bundled).toEqual(remote);
    expect(normalizeStoreModels(bundled)).toHaveLength(bundled.length);
    expect(new Set(bundled.map((model) => model.id)).size).toBe(bundled.length);
  });

  it('only advertises the GGUF engine used by this download flow', () => {
    for (const model of bundled) {
      expect(model.runtimes).toEqual(['llama-server']);
      expect(model.huggingfaceRepo).toMatch(/^[\w.-]+\/[\w.-]+$/);
      expect(model.name).not.toBe('');
      expect(model.variants.length).toBeGreaterThan(0);
      expect(new Set(model.variants.map((variant) => variant.quantization)).size).toBe(
        model.variants.length
      );
    }
  });

  it('puts smaller model weights first', () => {
    const sizes = bundled.map((model) =>
      Math.min(...model.variants.map((variant) => variant.fileSize))
    );
    expect(sizes).toEqual([...sizes].sort((a, b) => a - b));
  });

  it('lists safe filenames and every split GGUF shard in loading order', () => {
    const primaryFiles = new Set<string>();
    for (const model of bundled) {
      for (const variant of model.variants) {
        expect(Number.isSafeInteger(variant.fileSize)).toBe(true);
        expect(variant.fileSize).toBeGreaterThan(0);
        expect(variant.files.length).toBeGreaterThan(0);
        expect(new Set(variant.files).size).toBe(variant.files.length);
        const primary = variant.files[0].split('/').pop()!;
        expect(primaryFiles.has(primary)).toBe(false);
        primaryFiles.add(primary);

        for (const [index, file] of variant.files.entries()) {
          expect(file).toMatch(/^[\w./-]+\.gguf$/);
          expect(file.split('/')).not.toContain('..');
          const shard = file.match(/^(.*)-(\d{5})-of-(\d{5})\.gguf$/);
          if (variant.files.length > 1) {
            expect(shard).not.toBeNull();
            expect(Number(shard![2])).toBe(index + 1);
            expect(Number(shard![3])).toBe(variant.files.length);
            expect(shard![1]).toBe(variant.files[0].replace(/-\d{5}-of-\d{5}\.gguf$/, ''));
          } else {
            expect(shard).toBeNull();
          }
        }
      }
    }
  });
});
