import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const payload = {
  items: [{ id: 'article', title: 'AI news', source: 'AI Lab', url: 'https://example.com/news' }],
};
const response = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status });
let fetchFeed: typeof import('./feed').fetchFeed;
let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(async () => {
  vi.resetModules();
  ({ fetchFeed } = await import('./feed'));
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('news mirrors', () => {
  it('rejects unavailable and malformed sources, then keeps the working CDN preferred', async () => {
    fetchMock
      .mockResolvedValueOnce(response({}, 404))
      .mockResolvedValueOnce(response({ items: [{}] }))
      .mockImplementation(async () => response(payload));
    const signal = new AbortController().signal;
    expect(await fetchFeed('zh', signal)).toEqual(payload);
    expect(fetchMock.mock.calls[2][0]).toContain('cdn.jsdelivr.net/gh/SuYxh');
    await fetchFeed('zh', signal);
    expect(fetchMock.mock.calls[3][0]).toContain('cdn.jsdelivr.net/gh/SuYxh');
  });

  it('uses the bilingual fallback when the global feed is not yet deployed', async () => {
    fetchMock.mockImplementation(async (url: string) =>
      url.endsWith('latest-7d-en.json') ? response({}, 404) : response(payload)
    );
    expect(await fetchFeed('en', new AbortController().signal)).toEqual(payload);
    expect(fetchMock).toHaveBeenCalledTimes(4);
    expect(fetchMock.mock.calls[3][0]).toContain('latest-7d.json');
  });

  it('advances past a stalled mirror after ten seconds', async () => {
    vi.useFakeTimers();
    fetchMock
      .mockImplementationOnce(
        (_url, { signal }: { signal: AbortSignal }) =>
          new Promise((_resolve, reject) => {
            signal.addEventListener('abort', () => reject(new Error('timeout')));
          })
      )
      .mockImplementation(async () => response(payload));
    const pending = fetchFeed('zh', new AbortController().signal);
    await vi.advanceTimersByTimeAsync(10000);
    expect(await pending).toEqual(payload);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it('cancels the request without visiting another mirror or language fallback', async () => {
    const controller = new AbortController();
    fetchMock.mockImplementation(
      (_url, { signal }: { signal: AbortSignal }) =>
        new Promise((_resolve, reject) => {
          signal.addEventListener('abort', () => reject(new Error('cancelled')));
        })
    );
    const pending = fetchFeed('en', controller.signal);
    controller.abort();
    await expect(pending).rejects.toThrow();
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
