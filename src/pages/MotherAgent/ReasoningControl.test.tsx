import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { loadLocale, translate, type TKey } from '../../i18n';
import { ReasoningControl } from './ReasoningControl';
import type { ReasoningEffort } from '../../api/types';

let activePage = 'mother';
let locale = 'en';
let renderer: ReactTestRenderer | undefined;
const onChange = vi.fn();
const focus = vi.fn();
const levels: ReasoningEffort[] = ['low', 'high', 'max'];

vi.mock('../../hooks/useI18n', () => ({
  useI18n: () => ({ t: (key: TKey) => translate(key, locale) }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  activePage = 'mother';
  locale = 'en';
  vi.stubGlobal('document', new EventTarget());
});
afterEach(() => {
  if (renderer) act(() => renderer?.unmount());
  renderer = undefined;
  vi.unstubAllGlobals();
});

function render(value?: ReasoningEffort, disabled = false, options = levels, key = 'model') {
  const component = (
    <ReasoningControl
      key={JSON.stringify([key, disabled, activePage])}
      levels={options}
      value={value}
      disabled={disabled}
      onChange={onChange}
    />
  );
  act(() => {
    if (renderer) renderer.update(component);
    else
      renderer = create(component, {
        createNodeMock: (element) =>
          element.type === 'button' ? { focus } : { contains: () => false },
      });
  });
}

function open() {
  act(() => renderer!.root.findByType('button').props.onClick());
}

describe('Mother reasoning control', () => {
  it('opens on click and selects discrete supported levels including default', () => {
    render();
    expect(renderer!.root.findAllByProps({ role: 'dialog' })).toHaveLength(0);
    expect(onChange).not.toHaveBeenCalled();
    open();
    const slider = renderer!.root.findByType('input');
    expect(slider.props).toMatchObject({
      type: 'range',
      min: 0,
      max: 3,
      step: 1,
      value: 0,
      'aria-valuetext': 'Default',
    });
    act(() => slider.props.onChange({ currentTarget: { value: '2' } }));
    expect(onChange).toHaveBeenLastCalledWith('high');
    act(() => slider.props.onChange({ currentTarget: { value: '0' } }));
    expect(onChange).toHaveBeenLastCalledWith(undefined);
    render('max');
    expect(renderer!.root.findByType('input').props['aria-valuetext']).toBe('Max');
    expect(renderer!.root.findByType('button').props.className).toContain('text-cyber-accent');
    const markup = JSON.stringify(renderer!.toJSON());
    expect(markup).not.toContain('cursor-');
    expect(markup).not.toContain('title');
  });

  it('closes on Escape and outside interaction and removes listeners on unmount', () => {
    render('high');
    open();
    act(() => {
      document.dispatchEvent(Object.assign(new Event('keydown'), { key: 'Escape' }));
    });
    expect(renderer!.root.findAllByType('input')).toHaveLength(0);
    expect(focus).toHaveBeenCalledOnce();
    open();
    act(() => {
      document.dispatchEvent(new Event('mousedown'));
    });
    expect(renderer!.root.findAllByType('input')).toHaveLength(0);
    open();
    act(() => renderer?.unmount());
    renderer = undefined;
    act(() => {
      document.dispatchEvent(Object.assign(new Event('keydown'), { key: 'Escape' }));
    });
    expect(focus).toHaveBeenCalledOnce();
  });

  it('closes on pending, page exit, and model or conversation changes', () => {
    render();
    open();
    render(undefined, true);
    expect(renderer!.root.findAllByType('input')).toHaveLength(0);
    expect(renderer!.root.findByType('button').props.disabled).toBe(true);
    open();
    expect(renderer!.root.findAllByType('input')).toHaveLength(0);
    render();
    open();
    activePage = 'apps';
    render();
    activePage = 'mother';
    render();
    expect(renderer!.root.findAllByType('input')).toHaveLength(0);
    open();
    render(undefined, false, levels, 'new-model-or-conversation');
    expect(renderer!.root.findAllByType('input')).toHaveLength(0);
    render(undefined, false, []);
    expect(renderer!.root.findByType('button').props.disabled).toBe(true);
    open();
    expect(renderer!.root.findAllByType('input')).toHaveLength(0);
    expect(onChange).not.toHaveBeenCalled();
  });

  it('uses localized compact labels', async () => {
    await loadLocale('zh-Hans');
    locale = 'zh-Hans';
    render('max');
    expect(renderer!.root.findByType('button').props['aria-label']).toBe('推理强度: 极致');
    expect(renderer!.root.findByType('button').children).toEqual(['极致']);
    open();
    expect(renderer!.root.findByType('input').props['aria-valuetext']).toBe('极致');
  });
});
