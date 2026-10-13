import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { loadLocale, translate, type TKey } from '../../i18n';
import * as api from '../../api/tauri';
import { ConversationList } from './ConversationList';
import { MotherAgentMain } from './MotherAgentMain';
import { QuickCommandChips } from './QuickCommandChips';
import { ChatBubble } from '../../components/chat';
import type { MotherAgentCtx } from './types';

let state: Partial<MotherAgentCtx>;
let locale = 'en';
let activePage = 'mother';
let renderer: ReactTestRenderer | undefined;

vi.mock('./context', () => ({ useMotherAgent: () => state }));
vi.mock('../../hooks/useI18n', () => ({
  useI18n: () => ({ locale, t: (key: TKey) => translate(key, locale) }),
}));
vi.mock('../../hooks/useDocumentVisible', () => ({ useDocumentVisible: () => true }));
vi.mock('../../stores/navigationStore', () => ({
  useNavigationStore: (selector: (value: { activePage: string }) => unknown) =>
    selector({ activePage }),
}));
vi.mock('../../components/Toast', () => ({ useToast: () => ({ showToast: vi.fn() }) }));
vi.mock('../../components/RemoteModelSelector', () => ({ RemoteModelSelector: () => null }));
vi.mock('../../components/chat', () => ({
  ChatBubble: ({ content }: { content: string }) => <span>{content}</span>,
  ToolCallCard: () => null,
}));
vi.mock('../../api/tauri', () => ({
  getMotherHints: vi.fn(async () => '{"hints":[{"action":"showSpecs"}]}'),
  getLlmServerInfo: vi.fn(async () => ({ running: false })),
}));

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-12T09:00:00Z'));
  vi.clearAllMocks();
  vi.stubGlobal('window', { addEventListener: vi.fn(), removeEventListener: vi.fn() });
  vi.stubGlobal('requestAnimationFrame', vi.fn());
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    }
  );
  locale = 'en';
  activePage = 'mother';
  vi.stubGlobal('document', new EventTarget());
  state = {
    conversations: [
      {
        id: 'first',
        serverId: 'local',
        title: 'Fix my environment',
        createdAt: Date.now() - 86_400_000,
        updatedAt: Date.now() - 86_400_000,
        sessionKey: 'first',
        messages: [
          { type: 'user', text: 'Check it' },
          { type: 'assistant', text: 'Done' },
          { type: 'tool_call', id: 'tool', name: 'check', args: '{}', status: 'done' },
          { type: 'error', text: 'An earlier failure' },
        ],
      },
      {
        id: 'second',
        serverId: 'local',
        title: '',
        createdAt: Date.now(),
        updatedAt: Date.now(),
        sessionKey: 'second',
        messages: [],
      },
    ],
    activeConversationId: 'first',
    createConversation: vi.fn(),
    selectConversation: vi.fn(),
    deleteConversation: vi.fn(async () => {}),
    isProcessing: false,
    isResetting: false,
    setChatInput: vi.fn(),
    selectedServerId: 'local',
    models: [],
    chatInput: '',
    chatOutput: [],
    chatEndRef: { current: null },
    chatInputRef: { current: null },
    clearChat: vi.fn(),
    parasiteAgent: 'claudecode',
    parasiteAvailable: [],
    maDiskTotal: 0,
    loadOlderChat: vi.fn(async () => []),
    reasoningLevels: [],
    setReasoningEffort: vi.fn(),
  };
});

it('keeps the composer effort popup usable while running and closes it on reset, model changes, and page exit', () => {
  state = {
    ...state,
    parasiteAgent: null,
    reasoningLevels: ['low', 'high', 'max'],
    reasoningEffort: 'high',
  };
  act(() => {
    renderer = create(<MotherAgentMain />);
  });
  const open = () =>
    act(() =>
      renderer!.root.findByProps({ 'aria-label': 'Reasoning effort: High' }).props.onClick()
    );
  const slider = () => renderer!.root.findAllByProps({ type: 'range' });
  open();
  expect(slider()).toHaveLength(1);
  state = { ...state, isProcessing: true };
  act(() => renderer!.update(<MotherAgentMain />));
  expect(slider()).toHaveLength(1);
  expect(
    renderer!.root.findByProps({ 'aria-label': 'Reasoning effort: High' }).props.disabled
  ).toBe(false);
  act(() => slider()[0].props.onChange({ currentTarget: { value: '1' } }));
  expect(state.setReasoningEffort).toHaveBeenCalledExactlyOnceWith('low');
  state.setReasoningEffort = vi.fn();
  state = { ...state, isProcessing: false };
  act(() => renderer!.update(<MotherAgentMain />));
  expect(slider()).toHaveLength(1);
  state = { ...state, isResetting: true };
  act(() => renderer!.update(<MotherAgentMain />));
  expect(slider()).toHaveLength(0);
  state = { ...state, isResetting: false };
  act(() => renderer!.update(<MotherAgentMain />));
  expect(slider()).toHaveLength(0);
  open();
  state = { ...state, agentModel: 'another-model' };
  act(() => renderer!.update(<MotherAgentMain />));
  expect(slider()).toHaveLength(0);
  open();
  activePage = 'apps';
  act(() => renderer!.update(<MotherAgentMain />));
  expect(slider()).toHaveLength(0);
  activePage = 'mother';
  act(() => renderer!.update(<MotherAgentMain />));
  expect(slider()).toHaveLength(0);
  expect(state.setReasoningEffort).not.toHaveBeenCalled();
});

afterEach(() => {
  if (renderer) act(() => renderer?.unmount());
  renderer = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('Mother Agent conversation list', () => {
  it('shows the current conversation, saved titles, message count, and relative update time', () => {
    act(() => {
      renderer = create(<ConversationList />);
    });
    const buttons = renderer!.root.findAllByType('button');
    const markup = JSON.stringify(renderer!.toJSON());
    expect(buttons[1].props['aria-current']).toBe('true');
    expect(markup).toContain('Fix my environment');
    expect(markup).toContain('2 messages');
    expect(markup).toContain('1 day ago');
    expect(markup).toContain('0 messages');
    expect(markup).toContain('Just now');
    expect(buttons[3].props['aria-label']).toBe('New conversation');
  });

  it('starts or selects only on explicit clicks and blocks both actions while processing', () => {
    act(() => {
      renderer = create(<ConversationList />);
    });
    expect(state.createConversation).not.toHaveBeenCalled();
    expect(state.selectConversation).not.toHaveBeenCalled();
    let buttons = renderer!.root.findAllByType('button');
    act(() => buttons[0].props.onClick());
    act(() => buttons[3].props.onClick());
    expect(state.createConversation).toHaveBeenCalledTimes(1);
    expect(state.selectConversation).toHaveBeenCalledWith('second');
    state = { ...state, isProcessing: true };
    act(() => renderer!.update(<ConversationList />));
    buttons = renderer!.root.findAllByType('button');
    expect(buttons.every((button) => button.props.disabled)).toBe(true);
    act(() => buttons.forEach((button) => button.props.onClick({ stopPropagation: vi.fn() })));
    expect(state.createConversation).toHaveBeenCalledTimes(1);
    expect(state.selectConversation).toHaveBeenCalledTimes(1);
  });

  it('localizes metadata and refreshes its time while the list remains open', async () => {
    await loadLocale('zh-Hans');
    locale = 'zh-Hans';
    act(() => {
      renderer = create(<ConversationList />);
    });
    expect(JSON.stringify(renderer!.toJSON())).toContain('2 条消息');
    expect(JSON.stringify(renderer!.toJSON())).toContain('1天前');
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(JSON.stringify(renderer!.toJSON())).toContain('1分钟前');
  });

  it('deletes through a separate button without selecting the record and disables deletion while processing', () => {
    act(() => {
      renderer = create(<ConversationList />);
    });
    const deleteLabel = `${translate('btn.delete', locale)} ${translate('mother.newConversation', locale)}`;
    const deletion = renderer!.root.findByProps({ 'aria-label': deleteLabel });
    expect(deletion.parent?.type).toBe('div');
    const event = { stopPropagation: vi.fn() };
    act(() => deletion.props.onClick(event));
    expect(event.stopPropagation).toHaveBeenCalled();
    expect(state.deleteConversation).toHaveBeenCalledExactlyOnceWith('second');
    expect(state.selectConversation).not.toHaveBeenCalled();
    state = { ...state, isProcessing: true };
    act(() => renderer!.update(<ConversationList />));
    const disabled = renderer!.root.findByProps({ 'aria-label': deleteLabel });
    expect(disabled.props.disabled).toBe(true);
    act(() => disabled.props.onClick(event));
    expect(state.deleteConversation).toHaveBeenCalledTimes(1);
  });
});

describe('Mother Agent welcome screen', () => {
  it('keeps quick commands as editable input without sending a message', async () => {
    state = { ...state, sendMessage: vi.fn(), handleChatSend: vi.fn() };
    await act(async () => {
      renderer = create(<QuickCommandChips />);
    });
    const button = renderer!.root.findAllByType('button')[0];
    act(() => button.props.onClick());
    expect(state.setChatInput).toHaveBeenCalledWith('Show local machine specs');
    expect(state.sendMessage).not.toHaveBeenCalled();
    expect(state.handleChatSend).not.toHaveBeenCalled();
    expect(button.props.className).not.toContain('cursor-');
  });

  it('shows the existing logo and commands for an empty conversation and keeps the input', async () => {
    await act(async () => {
      renderer = create(<MotherAgentMain />);
    });
    expect(renderer!.root.findByType('img').props.src).toBe('/brand/bird.png');
    expect(JSON.stringify(renderer!.toJSON())).toContain('EchoBird');
    expect(JSON.stringify(renderer!.toJSON())).toContain('Show local machine specs');
    expect(renderer!.root.findByType('textarea').props.value).toBe('');
    state = { ...state, chatOutput: [{ type: 'user', text: 'Hello' }] };
    await act(async () => renderer!.update(<MotherAgentMain />));
    expect(renderer!.root.findAllByType(QuickCommandChips)).toHaveLength(0);
    expect(JSON.stringify(renderer!.toJSON())).toContain('Hello');
    expect(renderer!.root.findAllByType('textarea')).toHaveLength(1);
  });

  it('does not let delayed pagination from an old conversation change the newly selected one', async () => {
    const messages = Array.from({ length: 60 }, (_, i) => ({
      type: 'user' as const,
      text: `message-${i}`,
    }));
    state = { ...state, chatOutput: messages };
    const scrollContainer = {
      scrollTop: 0,
      scrollHeight: 1000,
      clientHeight: 300,
      children: [],
    };
    await act(async () => {
      renderer = create(<MotherAgentMain />, {
        createNodeMock: (element) => (element.props.onScroll ? scrollContainer : null),
      });
    });
    const scrollArea = renderer!.root.find((node) => typeof node.props.onScroll === 'function');
    act(() => scrollArea.props.onScroll());
    state = { ...state, activeConversationId: 'second' };
    await act(async () => renderer!.update(<MotherAgentMain />));
    act(() => {
      vi.advanceTimersByTime(300);
    });
    expect(renderer!.root.findAllByType(ChatBubble).map((bubble) => bubble.props.content)).toEqual(
      messages.slice(30).map((message) => message.text)
    );
    expect(api.getMotherHints).not.toHaveBeenCalled();
  });
});
