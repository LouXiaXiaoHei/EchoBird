import { describe, expect, it } from 'vitest';
import { accountProxyProviders } from './accountProxy';

describe('Kimi account registration', () => {
  it('offers one shared account provider for both regions and clients', () => {
    const providers = accountProxyProviders.filter((provider) => provider.name === 'Kimi');
    expect(providers).toEqual([{ id: 'kimi', name: 'Kimi', icon: 'kimidesktop.png' }]);
  });
});
