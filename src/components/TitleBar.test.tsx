import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { TitleBar } from './TitleBar';

const windowApi = vi.hoisted(() => ({
  isMaximized: vi.fn(async () => false),
  onResized: vi.fn(async () => vi.fn()),
  startDragging: vi.fn(async () => {}),
  minimize: vi.fn(async () => {}),
  maximize: vi.fn(async () => {}),
  destroy: vi.fn(async () => {}),
}));
const lifecycleApi = vi.hoisted(() => ({
  hideMainWindow: vi.fn(async () => {}),
  getSettings: vi.fn(),
}));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => windowApi }));
vi.mock('../api/tauri', () => lifecycleApi);
vi.mock('../utils/platform', () => ({ IS_MACOS: false }));
vi.mock('../hooks/useI18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock('./CloseWindowDialog', () => ({ CloseWindowDialog: () => null }));

let view: ReactTestRenderer;
beforeEach(() => {
  vi.clearAllMocks();
  lifecycleApi.getSettings.mockResolvedValue({
    closeWindowBehaviorSet: true,
    closeToTray: false,
  });
});
afterEach(() => act(() => view?.unmount()));

it('keeps left-button dragging on the blank title area and excludes nested controls', async () => {
  await act(async () => {
    view = create(<TitleBar />);
  });
  const down = view.root.findAllByType('div')[0].props.onMouseDown;
  const preventDefault = vi.fn();
  down({ button: 0, target: { closest: () => null }, preventDefault });
  expect(windowApi.startDragging).toHaveBeenCalledTimes(1);
  expect(preventDefault).toHaveBeenCalledTimes(1);
  down({ button: 0, target: { closest: () => ({}) }, preventDefault });
  down({ button: 2, target: { closest: () => null }, preventDefault });
  expect(windowApi.startDragging).toHaveBeenCalledTimes(1);
  expect(preventDefault).toHaveBeenCalledTimes(1);
});

it('keeps header actions and native window controls independent of dragging', async () => {
  const changelog = vi.fn(),
    feedback = vi.fn(),
    settings = vi.fn();
  await act(async () => {
    view = create(
      <TitleBar
        onChangelogClick={changelog}
        onFeedbackClick={feedback}
        onSettingsClick={settings}
      />
    );
  });
  for (const button of view.root.findAllByType('button')) {
    await act(async () => {
      await button.props.onClick();
    });
  }
  for (const action of [
    changelog,
    feedback,
    settings,
    lifecycleApi.hideMainWindow,
    windowApi.minimize,
    windowApi.maximize,
    windowApi.destroy,
  ]) {
    expect(action).toHaveBeenCalledTimes(1);
  }
  expect(windowApi.startDragging).not.toHaveBeenCalled();
});

it('uses the native visibility bridge when closing to the tray', async () => {
  lifecycleApi.getSettings.mockResolvedValue({
    closeWindowBehaviorSet: true,
    closeToTray: true,
  });
  await act(async () => {
    view = create(<TitleBar />);
  });
  const buttons = view.root.findAllByType('button');
  await act(async () => {
    await buttons[buttons.length - 1].props.onClick();
  });
  expect(lifecycleApi.hideMainWindow).toHaveBeenCalledTimes(1);
  expect(windowApi.destroy).not.toHaveBeenCalled();
  expect(windowApi.startDragging).not.toHaveBeenCalled();
});
