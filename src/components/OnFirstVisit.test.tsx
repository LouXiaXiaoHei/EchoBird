import { lazy, useEffect, useState } from 'react';
import { act, create } from 'react-test-renderer';
import { expect, it, vi } from 'vitest';
import { OnFirstVisit } from './OnFirstVisit';
it('does not load before entry and preserves page state and effects across navigation', async () => {
  const mount = vi.fn(),
    cleanup = vi.fn();
  function Page() {
    const [value, setValue] = useState('draft');
    useEffect(() => {
      mount();
      return cleanup;
    }, []);
    return <input value={value} onChange={() => setValue('edited')} />;
  }
  const load = vi.fn(async () => ({ default: Page }));
  const Deferred = lazy(load);
  const content = (active: boolean) => (
    <OnFirstVisit active={active}>
      <Deferred />
    </OnFirstVisit>
  );
  let view!: ReturnType<typeof create>;
  await act(async () => {
    view = create(content(false));
  });
  expect(load).not.toHaveBeenCalled();
  await act(async () => view.update(content(true)));
  act(() => view.root.findByType('input').props.onChange());
  await act(async () => view.update(content(false)));
  expect(cleanup).not.toHaveBeenCalled();
  await act(async () => view.update(content(true)));
  expect(view.root.findByType('input').props.value).toBe('edited');
  expect(load).toHaveBeenCalledOnce();
  expect(mount).toHaveBeenCalledOnce();
  act(() => view.unmount());
  expect(cleanup).toHaveBeenCalledOnce();
});
