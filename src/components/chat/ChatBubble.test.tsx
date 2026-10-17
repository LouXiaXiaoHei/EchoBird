import { act, create } from 'react-test-renderer';
import { expect, it, vi } from 'vitest';
import { ChatBubble } from './ChatBubble';
const calls = vi.hoisted(() => ({ markdown: vi.fn() }));
vi.mock('../../hooks/useI18n', () => ({
  useI18n: () => ({ t: (key: string) => key, locale: 'en' }),
}));
vi.mock('../cards/ModelCard', () => ({ getModelIcon: () => null }));
vi.mock('../../pages/MotherAgent/mdComponents', () => ({ mdComponents: {} }));
vi.mock('react-markdown', async (original) => {
  const real = await original<typeof import('react-markdown')>();
  return {
    default: (props: React.ComponentProps<typeof real.default>) => {
      calls.markdown();
      return <real.default {...props} />;
    },
  };
});
it('parses historical Markdown once while the current answer streams', () => {
  const transcript = (chunk: number) => (
    <>
      {Array.from({ length: 14 }, (_, index) => (
        <ChatBubble key={index} role="assistant" content={`## Saved answer ${index} **bold**`} />
      ))}
      <ChatBubble key="tail" role="assistant" content={'chunk '.repeat(chunk)} isStreaming />
    </>
  );
  let view!: ReturnType<typeof create>;
  act(() => {
    view = create(transcript(1));
  });
  for (let chunk = 2; chunk <= 21; chunk++) act(() => view.update(transcript(chunk)));
  expect(calls.markdown).toHaveBeenCalledTimes(35);
  act(() => view.unmount());
});
