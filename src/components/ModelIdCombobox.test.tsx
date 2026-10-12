import { useState, type ComponentProps } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ModelIdCombobox } from './ModelIdCombobox';

const { portal } = vi.hoisted(() => ({
  portal: vi.fn<(children: unknown, target: unknown) => unknown>((children) => children),
}));
vi.mock('react-dom', () => ({ createPortal: portal }));

type Props = ComponentProps<typeof ModelIdCombobox>;
let renderer: ReactTestRenderer;
let props: Props;
const blur = vi.fn();
const addEventListener = vi.fn();
const removeEventListener = vi.fn();
const body = {};

function Harness(input: Props) {
  const [value, setValue] = useState(input.value);
  return (
    <ModelIdCombobox
      {...input}
      value={value}
      onChange={(next) => {
        input.onChange(next);
        setValue(next);
      }}
    />
  );
}

function mount(extra: Partial<Props> = {}) {
  props = { value: '', onChange: vi.fn(), ...extra };
  act(() => {
    renderer = create(<Harness {...props} />, {
      createNodeMock: (element) =>
        element.type === 'input'
          ? {
              blur,
              getBoundingClientRect: () => ({ left: 12, width: 240, top: 100, bottom: 130 }),
            }
          : { contains: () => false },
    });
  });
}

function update(extra: Partial<Props>) {
  props = { ...props, ...extra };
  act(() => renderer.update(<Harness {...props} />));
}

const input = () => renderer.root.findByType('input');
const menus = () => renderer.root.findAll((node) => node.props.style?.position === 'fixed');
const options = () => menus()[0].findAll((node) => typeof node.props.onMouseDown === 'function');
const button = (label: string) => renderer.root.findByProps({ 'aria-label': label });
const key = (name: string) => {
  const preventDefault = vi.fn();
  act(() => input().props.onKeyDown({ key: name, preventDefault }));
  return preventDefault;
};

beforeEach(() => {
  vi.stubGlobal('window', {
    innerHeight: 800,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  });
  vi.stubGlobal('document', { body, addEventListener, removeEventListener });
});

afterEach(() => {
  act(() => renderer?.unmount());
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

it('preserves arbitrary API model input and its plain-text paste action', () => {
  const paste = vi.fn();
  mount({ onPaste: paste, pasteLabel: 'Paste model' });
  expect(input().props.readOnly).toBe(false);
  expect(input().props.style).toBeUndefined();
  act(() => input().props.onChange({ target: { value: 'custom-model-id' } }));
  expect(props.onChange).toHaveBeenCalledWith('custom-model-id');
  expect(input().props.value).toBe('custom-model-id');
  act(() => renderer.root.findByType('button').props.onClick());
  expect(paste).toHaveBeenCalledTimes(1);
  expect(portal).not.toHaveBeenCalled();

  update({ disabled: true });
  act(() => renderer.root.findByType('button').props.onClick());
  expect(paste).toHaveBeenCalledTimes(1);
});

it('preserves API browse, search, keyboard selection and the positioned body portal', () => {
  mount({ value: 'gpt-a', options: ['gpt-a', 'gpt-b', 'claude-a', 'gpt-a'] });
  act(() => input().props.onFocus());
  expect(options().map((node) => node.children[0])).toEqual(['gpt-a', 'gpt-b', 'claude-a']);
  expect(portal.mock.calls[portal.mock.calls.length - 1]?.[1]).toBe(body);
  expect(menus()[0].props.style).toMatchObject({ left: 12, width: 240, top: 131 });
  expect(menus()[0].props.className).toContain('z-[9999]');
  act(() => input().props.onChange({ target: { value: 'gpt' } }));
  expect(options().map((node) => node.children[0])).toEqual(['gpt-a', 'gpt-b']);
  key('ArrowDown');
  key('ArrowDown');
  expect(key('Enter')).toHaveBeenCalledOnce();
  expect(props.onChange).toHaveBeenLastCalledWith('gpt-b');
  expect(menus()).toHaveLength(0);
  expect(blur).toHaveBeenCalledOnce();
  act(() => input().props.onClick());
  expect(options()).toHaveLength(3);
  key('Escape');
  expect(menus()).toHaveLength(0);
  act(() => input().props.onClick());
  const outside = addEventListener.mock.calls.find(([name]) => name === 'mousedown')?.[1];
  act(() => outside({ target: {} }));
  expect(menus()).toHaveLength(0);
});

it('shows labels while select-only choices preserve distinct IDs with duplicate labels', () => {
  const paste = vi.fn();
  mount({
    value: 'first-id',
    options: ['first-id', 'second-id', 'unlabelled-id'],
    optionLabels: { 'first-id': 'Same account', 'second-id': 'Same account' },
    selectOnly: true,
    ariaLabel: 'Account',
    onPaste: paste,
    pasteLabel: 'Paste account',
  });
  expect(input().props.readOnly).toBe(true);
  expect(input().props.style).toEqual({ cursor: 'default' });
  expect(input().props['aria-label']).toBe('Account');
  expect(input().props.value).toBe('Same account');
  act(() => input().props.onChange({ target: { value: 'typed account' } }));
  expect(props.onChange).not.toHaveBeenCalled();
  expect(renderer.root.findAllByType('button')).toHaveLength(1);
  act(() => input().props.onFocus());
  expect(options().map((node) => node.children[0])).toEqual([
    'Same account',
    'Same account',
    'unlabelled-id',
  ]);
  const preventDefault = vi.fn();
  act(() => options()[1].props.onMouseDown({ preventDefault }));
  expect(props.onChange).toHaveBeenCalledExactlyOnceWith('second-id');
  expect(preventDefault).toHaveBeenCalledOnce();
  expect(paste).not.toHaveBeenCalled();
});

it('searches display labels and still commits the selected ID', () => {
  mount({
    options: ['opaque-one', 'opaque-two'],
    optionLabels: { 'opaque-one': 'Claude account', 'opaque-two': 'Codex account' },
  });
  act(() => input().props.onChange({ target: { value: 'codex' } }));
  expect(options().map((node) => node.children[0])).toEqual(['Codex account']);
  key('ArrowDown');
  key('Enter');
  expect(props.onChange).toHaveBeenLastCalledWith('opaque-two');
  expect(input().props.value).toBe('Codex account');
});

it('allows select-only keyboard selection but disables the chevron without options', () => {
  mount({ options: ['one', 'two'], selectOnly: true });
  key('ArrowDown');
  key('ArrowDown');
  key('ArrowDown');
  key('Enter');
  expect(props.onChange).toHaveBeenCalledExactlyOnceWith('two');
  update({ options: [], placeholder: '—' });
  const chevron = button('Toggle model id suggestions');
  expect(chevron.props.disabled).toBe(true);
  expect(input().props.placeholder).toBe('—');
  act(() => {
    input().props.onFocus();
    input().props.onClick();
    chevron.props.onClick();
  });
  key('ArrowDown');
  expect(menus()).toHaveLength(0);
  expect(props.onChange).toHaveBeenCalledOnce();
});

it('refreshes independently of menu state and selection, with a separate busy lock', () => {
  const refresh = vi.fn();
  mount({
    value: 'one',
    options: ['one', 'two'],
    selectOnly: true,
    onRefresh: refresh,
    refreshLabel: 'Read models',
  });
  const preventDefault = vi.fn();
  const stopPropagation = vi.fn();
  act(() => {
    button('Read models').props.onMouseDown({ preventDefault });
    button('Read models').props.onClick({ stopPropagation });
  });
  expect(refresh).toHaveBeenCalledOnce();
  expect(preventDefault).toHaveBeenCalledOnce();
  expect(stopPropagation).toHaveBeenCalledOnce();
  expect(menus()).toHaveLength(0);
  expect(props.onChange).not.toHaveBeenCalled();
  expect(button('Read models').props.className).toContain('right-7');
  expect(button('Toggle model id suggestions').props.className).toContain('right-1.5');
  expect(input().props.className).toContain('pr-14');
  act(() => input().props.onFocus());
  act(() => button('Read models').props.onClick({ stopPropagation }));
  expect(refresh).toHaveBeenCalledTimes(2);
  expect(options()).toHaveLength(2);
  expect(input().props.value).toBe('one');
  update({ refreshing: true });
  expect(button('Read models').props.disabled).toBe(true);
  act(() => button('Read models').props.onClick({ stopPropagation }));
  expect(refresh).toHaveBeenCalledTimes(2);
  expect(options()).toHaveLength(2);
});

it('closes an open disabled field and prevents all selection or refresh callbacks', () => {
  const refresh = vi.fn();
  mount({ options: ['one'], onRefresh: refresh, refreshLabel: 'Read models' });
  act(() => input().props.onFocus());
  expect(menus()).toHaveLength(1);
  update({ disabled: true });
  expect(input().props.disabled).toBe(true);
  expect(menus()).toHaveLength(0);
  act(() => {
    input().props.onChange({ target: { value: 'typed' } });
    input().props.onFocus();
    input().props.onClick();
    button('Toggle model id suggestions').props.onClick();
    button('Read models').props.onClick({ stopPropagation: vi.fn() });
  });
  key('ArrowDown');
  key('Enter');
  expect(props.onChange).not.toHaveBeenCalled();
  expect(refresh).not.toHaveBeenCalled();
  update({ disabled: false });
  expect(menus()).toHaveLength(0);
});

it('does not commit an out-of-range option after the directory changes', () => {
  mount({ options: ['one', 'two'], selectOnly: true });
  act(() => input().props.onFocus());
  key('ArrowDown');
  key('ArrowDown');
  update({ options: ['one'] });
  key('Enter');
  expect(props.onChange).not.toHaveBeenCalled();
  key('ArrowUp');
  key('Enter');
  expect(props.onChange).toHaveBeenCalledExactlyOnceWith('one');
});
