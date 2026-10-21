import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, expect, it, vi } from 'vitest';
import type { AppManagerContextType } from './context';
import { DevinAccountSection } from './DevinAccountSection';
let state: AppManagerContextType;
let renderer: ReactTestRenderer | undefined;
vi.mock('./context', () => ({ useAppManager: () => state }));
vi.mock('../../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
afterEach(() => {
  act(() => renderer?.unmount());
  renderer = undefined;
});
function fixture() {
  const account = {
    id: 'saved',
    email: 'user@example.test',
    active: true,
    plan: 'Max',
    quotaWindows: [
      { label: '1 day', remainingPercent: 0 },
      { label: '7 days', remainingPercent: 42 },
    ],
  };
  const group = {
    accounts: [account],
    selectedId: account.id,
    busy: false,
    remainingSeconds: 0,
    refreshing: new Set<string>(),
    authorizationFailedIds: new Set<string>(),
    select: vi.fn(),
    add: vi.fn(),
    refresh: vi.fn(),
    remove: vi.fn(),
    cancelLogin: vi.fn(),
  };
  state = { devinAccounts: group } as unknown as AppManagerContextType;
  return { account, group };
}
it('shows plan once, both quota windows and isolated refresh/delete actions', () => {
  const { account, group } = fixture();
  act(() => {
    renderer = create(<DevinAccountSection />);
  });
  const html = renderToStaticMarkup(<DevinAccountSection />);
  expect(html.match(/Max/g)).toHaveLength(1);
  expect(html).toContain('0%');
  expect(html).toContain('42%');
  expect(html).toContain('24h');
  expect(html).toContain('7d');
  expect(html).toContain('·');
  expect(html).not.toContain('1 day');
  expect(html).not.toContain('7 days');
  expect(html).not.toContain('cursor-pointer');
  const stopPropagation = vi.fn();
  act(() =>
    renderer!.root
      .findByProps({ 'aria-label': `agent.refreshAccount ${account.email}` })
      .props.onClick({ stopPropagation })
  );
  act(() =>
    renderer!.root
      .findByProps({ 'aria-label': `btn.delete ${account.email}` })
      .props.onClick({ stopPropagation })
  );
  expect(group.refresh).toHaveBeenCalledExactlyOnceWith(account);
  expect(group.remove).toHaveBeenCalledExactlyOnceWith(account);
  expect(group.select).not.toHaveBeenCalled();
});
it('preserves unknown quota and disables the shared refresh control while pending', () => {
  const { account, group } = fixture();
  account.quotaWindows = [];
  group.refreshing.add(account.id);
  act(() => {
    renderer = create(<DevinAccountSection />);
  });
  expect(
    renderer!.root.findByProps({ 'aria-label': `agent.refreshAccount ${account.email}` }).props
      .disabled
  ).toBe(true);
  expect(renderToStaticMarkup(<DevinAccountSection />)).toContain('—');
});
