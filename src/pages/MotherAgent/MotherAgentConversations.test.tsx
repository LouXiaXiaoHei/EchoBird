import React, { useLayoutEffect } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from '../../api/tauri';
import type { AgentEvent, ModelConfig, ParasiteEvent } from '../../api/types';
import { useNavigationStore } from '../../stores/navigationStore';
import { MotherAgentProvider } from './MotherAgentProvider';
import { useMotherAgent } from './context';
import type { MotherAgentCtx } from './types';

vi.mock('../../api/tauri', () => ({
  getModels: vi.fn(),
  loadSSHServers: vi.fn(),
  saveSSHServer: vi.fn(),
  removeSSHServerFromDisk: vi.fn(),
  parasiteListInstalled: vi.fn(),
  listenAgentEvents: vi.fn(),
  listenParasiteEvents: vi.fn(),
  sendAgentMessage: vi.fn(),
  abortAgent: vi.fn(),
  resetAgent: vi.fn(),
  parasiteSendMessage: vi.fn(),
  parasiteAbort: vi.fn(),
  parasiteReset: vi.fn(),
}));
vi.mock('../../hooks/useI18n', () => ({
  useI18n: () => ({ t: (key: string) => key, locale: 'en' }),
}));
vi.mock('../../components/ConfirmDialog', () => ({ useConfirm: () => confirm }));
vi.mock('../../components/Toast', () => ({ useToast: () => ({ showToast }) }));

const confirm = vi.fn();
const showToast = vi.fn();

const model: ModelConfig = {
  internalId: 'test-model',
  name: 'Test model',
  modelId: 'test-model-api',
  baseUrl: 'https://example.test/v1',
  apiKey: 'fixture-key',
};
let state: MotherAgentCtx;
let renderer: ReactTestRenderer | undefined;
let agentEvent: (event: AgentEvent) => void;
let parasiteEvent: (event: ParasiteEvent) => void;
let stored: Map<string, string>;

function Harness() {
  const result = useMotherAgent();
  useLayoutEffect(() => {
    state = result;
  });
  return null;
}

async function mount() {
  await act(async () => {
    renderer = create(
      <MotherAgentProvider>
        <Harness />
      </MotherAgentProvider>
    );
  });
}

function activeSession() {
  return state.conversations.find((c) => c.id === state.activeConversationId)!.sessionKey;
}

async function send(text: string) {
  await act(async () => {
    state.sendMessage(text);
  });
}

function emitAgent(event: AgentEvent) {
  const request = vi.mocked(api.sendAgentMessage).mock.lastCall?.[0];
  act(() => agentEvent({ ...event, request_id: event.request_id ?? request?.request_id }));
}

function emitParasite(event: ParasiteEvent) {
  const request = vi.mocked(api.parasiteSendMessage).mock.lastCall?.[0];
  act(() => parasiteEvent({ ...event, request_id: event.request_id ?? request?.requestId }));
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-12T09:00:00Z'));
  vi.resetAllMocks();
  confirm.mockResolvedValue(true);
  stored = new Map([['echobird_agent_model', model.internalId]]);
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => stored.set(key, value),
    removeItem: (key: string) => stored.delete(key),
    key: (index: number) => [...stored.keys()][index] ?? null,
    get length() {
      return stored.size;
    },
  });
  vi.stubGlobal('window', new EventTarget());
  let sequence = 0;
  vi.stubGlobal('crypto', { randomUUID: () => `conversation-${++sequence}` });
  useNavigationStore.setState({
    activePage: 'mother',
    motherPrefill: undefined,
    agentRunning: false,
  });
  vi.mocked(api.getModels).mockResolvedValue([model]);
  vi.mocked(api.loadSSHServers).mockResolvedValue([
    { id: 'remote', host: '192.0.2.1', port: 22, username: 'tester', password: '' },
  ]);
  vi.mocked(api.parasiteListInstalled).mockResolvedValue(['claudecode']);
  vi.mocked(api.listenAgentEvents).mockImplementation(async (handler) => {
    agentEvent = handler;
    return vi.fn<() => void>();
  });
  vi.mocked(api.listenParasiteEvents).mockImplementation(async (handler) => {
    parasiteEvent = handler;
    return vi.fn<() => void>();
  });
  vi.mocked(api.sendAgentMessage).mockResolvedValue('ok');
  vi.mocked(api.abortAgent).mockResolvedValue(true);
  vi.mocked(api.resetAgent).mockResolvedValue('ok');
  vi.mocked(api.parasiteSendMessage).mockResolvedValue(undefined);
  vi.mocked(api.parasiteAbort).mockResolvedValue(true);
  vi.mocked(api.parasiteReset).mockResolvedValue(undefined);
});

afterEach(() => {
  if (renderer) act(() => renderer?.unmount());
  renderer = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('Mother Agent saved conversation workflow', () => {
  it('deletes only the confirmed record and preserves the active conversation and draft', async () => {
    await mount();
    const deleted = state.activeConversationId;
    const deletedKey = activeSession();
    act(() => state.createConversation());
    const current = state.activeConversationId;
    await send('Keep current content');
    emitAgent({ type: 'done', conversation_id: activeSession() });
    act(() => state.setChatInput('Keep current draft'));
    const messages = state.chatOutput;
    await act(async () => {
      await state.deleteConversation(deleted);
    });
    expect(confirm).toHaveBeenCalledOnce();
    expect(api.resetAgent).toHaveBeenCalledExactlyOnceWith(deletedKey);
    expect(api.parasiteReset).toHaveBeenCalledExactlyOnceWith('claudecode', deletedKey);
    expect(state.activeConversationId).toBe(current);
    expect(state.chatOutput).toEqual(messages);
    expect(state.chatInput).toBe('Keep current draft');
    expect(state.conversations.some((c) => c.id === deleted)).toBe(false);
    expect(showToast).not.toHaveBeenCalled();
  });

  it('does nothing on a cancelled delete confirmation and blocks repeat actions while it is open', async () => {
    await mount();
    const current = state.activeConversationId;
    const decision = deferred<boolean>();
    confirm.mockReturnValueOnce(decision.promise);
    const callbacks = state;
    let task!: Promise<void>;
    act(() => {
      task = callbacks.deleteConversation(current);
      void callbacks.deleteConversation(current);
      callbacks.createConversation();
      callbacks.selectServer('remote');
      callbacks.sendMessage('Blocked');
    });
    expect(confirm).toHaveBeenCalledOnce();
    expect(state.isProcessing).toBe(true);
    expect(state.activeConversationId).toBe(current);
    await act(async () => {
      decision.resolve(false);
      await task;
    });
    expect(state.isProcessing).toBe(false);
    expect(state.conversations).toHaveLength(1);
    expect(api.resetAgent).not.toHaveBeenCalled();
    expect(api.parasiteReset).not.toHaveBeenCalled();
    expect(api.sendAgentMessage).not.toHaveBeenCalled();
  });

  it('preserves saved records and shows an error when deletion cleanup fails', async () => {
    await mount();
    const current = state.activeConversationId;
    vi.mocked(api.resetAgent).mockRejectedValueOnce(new Error('Cannot delete saved context'));
    await act(async () => {
      await state.deleteConversation(current);
    });
    expect(state.activeConversationId).toBe(current);
    expect(state.conversations).toHaveLength(1);
    expect(api.parasiteReset).not.toHaveBeenCalled();
    expect(state.chatOutput).toEqual([]);
    expect(showToast).toHaveBeenCalledExactlyOnceWith(
      'error',
      'Error: Cannot delete saved context'
    );
    expect(state.isProcessing).toBe(false);
  });

  it('blocks deletion during generation and clears the draft only when deleting the active conversation', async () => {
    await mount();
    const current = state.activeConversationId;
    await send('Pending turn');
    await act(async () => {
      await state.deleteConversation(current);
    });
    expect(confirm).not.toHaveBeenCalled();
    emitAgent({ type: 'done', conversation_id: activeSession() });
    act(() => state.setChatInput('Old draft'));
    await act(async () => {
      await state.deleteConversation(current);
    });
    expect(state.activeConversationId).not.toBe(current);
    expect(state.chatInput).toBe('');
    expect(state.chatOutput).toEqual([]);
    expect(state.conversations).toHaveLength(1);
  });

  it('keeps mount, page navigation, new conversations, and selection free of execution or resets', async () => {
    await mount();
    const first = state.activeConversationId;
    act(() => useNavigationStore.getState().setActivePage('apps'));
    act(() => useNavigationStore.getState().setActivePage('mother'));
    act(() => state.createConversation());
    const second = state.activeConversationId;
    expect(second).not.toBe(first);
    act(() => state.selectConversation(first));
    expect(state.activeConversationId).toBe(first);
    act(() => state.selectServer('remote'));
    expect(state.selectedServerId).toBe('remote');
    act(() => state.selectServer('local'));
    expect(state.selectedServerId).toBe('local');
    for (const method of [
      api.sendAgentMessage,
      api.abortAgent,
      api.resetAgent,
      api.parasiteSendMessage,
      api.parasiteAbort,
      api.parasiteReset,
      api.saveSSHServer,
    ])
      expect(method).not.toHaveBeenCalled();
    expect(api.getModels).toHaveBeenCalled();
    expect(state.conversations).toHaveLength(2);
  });

  it('sends independent session keys while preserving the selected server as the execution target', async () => {
    await mount();
    act(() => state.selectServer('remote'));
    const firstId = state.activeConversationId;
    const firstKey = activeSession();
    await send('First prompt');
    expect(api.sendAgentMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({ conversation_id: firstKey, server_ids: ['remote'] })
    );
    expect(firstKey).not.toBe('remote');
    emitAgent({ type: 'text_delta', text: 'First reply', conversation_id: firstKey });
    emitAgent({ type: 'done', conversation_id: firstKey });
    act(() => state.createConversation());
    const secondKey = activeSession();
    expect(secondKey).not.toBe(firstKey);
    expect(state.chatOutput).toEqual([]);
    await send('Second prompt');
    emitAgent({ type: 'text_delta', text: 'Second reply', conversation_id: secondKey });
    emitAgent({ type: 'done', conversation_id: secondKey });
    expect(api.sendAgentMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({ conversation_id: secondKey, server_ids: ['remote'] })
    );
    act(() => state.selectConversation(firstId));
    expect(state.chatOutput.map((m) => 'text' in m && m.text)).toEqual([
      'First prompt',
      'First reply',
    ]);
    expect(state.conversations.find((c) => c.sessionKey === secondKey)?.messages).toEqual([
      { type: 'user', text: 'Second prompt', chips: undefined },
      { type: 'assistant', text: 'Second reply' },
    ]);
    await act(async () => {
      await state.clearChat();
    });
    expect(api.resetAgent).toHaveBeenLastCalledWith(firstKey);
    expect(state.chatOutput).toEqual([]);
    expect(state.conversations.find((c) => c.sessionKey === secondKey)?.messages).toHaveLength(2);
  });

  it('blocks new, select, server changes, reset, and duplicate send even before the pending render', async () => {
    await mount();
    const first = state.activeConversationId;
    act(() => state.createConversation());
    const current = state.activeConversationId;
    const callbacks = state;
    await act(async () => {
      callbacks.sendMessage('Running');
      callbacks.createConversation();
      callbacks.selectConversation(first);
      callbacks.selectServer('remote');
      callbacks.clearChat();
      callbacks.removeSSHServer('remote');
      callbacks.sendMessage('Duplicate');
    });
    expect(state.isProcessing).toBe(true);
    expect(state.activeConversationId).toBe(current);
    expect(state.selectedServerId).toBe('local');
    expect(state.conversations).toHaveLength(2);
    expect(state.sshServers).toHaveLength(1);
    expect(api.sendAgentMessage).toHaveBeenCalledTimes(1);
    expect(api.resetAgent).not.toHaveBeenCalled();
    expect(api.removeSSHServerFromDisk).not.toHaveBeenCalled();
    act(() => {
      state.createConversation();
      state.selectConversation(first);
      state.selectServer('remote');
    });
    expect(state.activeConversationId).toBe(current);
  });

  it('ignores old, unscoped, and wrong-engine events while accepting replies scoped to the active run', async () => {
    await mount();
    const firstKey = activeSession();
    await send('Old prompt');
    emitAgent({ type: 'done', conversation_id: firstKey });
    act(() => state.createConversation());
    const currentKey = activeSession();
    await send('Current prompt');
    emitAgent({ type: 'text_delta', text: 'Old reply', conversation_id: firstKey });
    emitAgent({ type: 'error', message: 'Old failure', conversation_id: firstKey });
    emitAgent({ type: 'done', conversation_id: firstKey });
    emitAgent({ type: 'text_delta', text: 'Unscoped reply' });
    act(() =>
      agentEvent({ type: 'text_delta', text: 'Missing request ID', conversation_id: currentKey })
    );
    emitParasite({ type: 'text_delta', text: 'Wrong engine', conversation_id: currentKey });
    expect(state.isProcessing).toBe(true);
    expect(state.chatOutput).toEqual([{ type: 'user', text: 'Current prompt', chips: undefined }]);
    emitAgent({ type: 'text_delta', text: 'Current reply', conversation_id: currentKey });
    expect(state.chatOutput[state.chatOutput.length - 1]).toEqual({
      type: 'assistant',
      text: 'Current reply',
    });
    emitAgent({ type: 'done', conversation_id: currentKey });
    expect(state.isProcessing).toBe(false);
  });

  it('keeps late events and invocation rejection from an aborted run out of a later conversation', async () => {
    const pending = deferred<string>();
    vi.mocked(api.sendAgentMessage).mockReturnValueOnce(pending.promise);
    await mount();
    const firstKey = activeSession();
    await send('Cancelled prompt');
    act(() => {
      state.abortAgent();
      state.abortAgent();
    });
    expect(api.abortAgent).toHaveBeenCalledExactlyOnceWith(firstKey);
    expect(state.chatOutput.filter((m) => m.type === 'cancelled')).toHaveLength(1);
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    expect(state.isProcessing).toBe(false);
    act(() => state.createConversation());
    const currentKey = activeSession();
    await send('Current prompt');
    await act(async () => pending.reject(new Error('Late invocation failure')));
    emitAgent({ type: 'text_delta', text: 'Late cancelled reply', conversation_id: firstKey });
    emitAgent({ type: 'done', conversation_id: firstKey });
    expect(state.isProcessing).toBe(true);
    expect(state.chatOutput).toEqual([{ type: 'user', text: 'Current prompt', chips: undefined }]);
    emitAgent({ type: 'text_delta', text: 'Accepted reply', conversation_id: currentKey });
    expect(state.chatOutput[state.chatOutput.length - 1]).toEqual({
      type: 'assistant',
      text: 'Accepted reply',
    });
  });

  it('keeps no-model and explicit invocation failures in the conversation that sent them', async () => {
    await mount();
    const first = state.activeConversationId;
    act(() => state.setAgentModel(null));
    await send('Without model');
    expect(api.sendAgentMessage).not.toHaveBeenCalled();
    expect(state.chatOutput[state.chatOutput.length - 1]).toEqual({
      type: 'error',
      text: '',
      i18nKey: 'error.noModelSelected',
    });
    expect(state.isProcessing).toBe(false);
    act(() => state.createConversation());
    act(() => state.setAgentModel(model.internalId));
    vi.mocked(api.sendAgentMessage).mockRejectedValueOnce(new Error('Fixture provider failure'));
    await send('Failing prompt');
    expect(state.chatOutput[state.chatOutput.length - 1]).toEqual({
      type: 'error',
      text: 'Error: Fixture provider failure',
    });
    expect(state.isProcessing).toBe(false);
    act(() => state.selectConversation(first));
    expect(state.chatOutput).toHaveLength(2);
    expect(state.chatOutput[0]).toEqual({ type: 'user', text: 'Without model', chips: undefined });
    expect(api.sendAgentMessage).toHaveBeenCalledTimes(1);
  });

  it('preserves the transcript and blocks conversation actions until explicit reset succeeds', async () => {
    await mount();
    const first = state.activeConversationId;
    act(() => state.createConversation());
    const current = state.activeConversationId;
    const session = activeSession();
    await send('Saved prompt');
    emitAgent({ type: 'text_delta', text: 'Saved reply', conversation_id: session });
    emitAgent({ type: 'done', conversation_id: session });
    const pending = deferred<string>();
    vi.mocked(api.resetAgent).mockReturnValueOnce(pending.promise);
    const callbacks = state;
    act(() => {
      callbacks.clearChat();
      callbacks.createConversation();
      callbacks.selectConversation(first);
      callbacks.selectServer('remote');
      callbacks.sendMessage('Blocked during reset');
      callbacks.clearChat();
    });
    expect(state.activeConversationId).toBe(current);
    expect(state.chatOutput).toHaveLength(2);
    expect(state.isProcessing).toBe(true);
    expect(api.resetAgent).toHaveBeenCalledExactlyOnceWith(session);
    expect(api.sendAgentMessage).toHaveBeenCalledTimes(1);
    await act(async () => pending.resolve('ok'));
    expect(state.chatOutput).toEqual([]);
    expect(state.isProcessing).toBe(false);
    act(() => state.selectConversation(first));
    expect(state.activeConversationId).toBe(first);
  });

  it('keeps the saved transcript visible when an explicit backend reset fails', async () => {
    await mount();
    const current = state.activeConversationId;
    const session = activeSession();
    await send('Keep this prompt');
    emitAgent({ type: 'text_delta', text: 'Keep this reply', conversation_id: session });
    emitAgent({ type: 'done', conversation_id: session });
    vi.mocked(api.resetAgent).mockRejectedValueOnce(new Error('Cannot reset a running session'));
    await act(async () => {
      await state.clearChat();
    });
    expect(state.activeConversationId).toBe(current);
    expect(state.chatOutput).toEqual([
      { type: 'user', text: 'Keep this prompt', chips: undefined },
      { type: 'assistant', text: 'Keep this reply' },
      { type: 'error', text: 'Error: Cannot reset a running session' },
    ]);
    expect(state.isProcessing).toBe(false);
  });

  it.each(['mother', 'connect'] as const)(
    'ignores late %s events from an aborted turn after restarting the same conversation',
    async (engine) => {
      if (engine === 'connect') stored.set('echobird_parasite_agent', 'claudecode');
      await mount();
      const conversationId = state.activeConversationId;
      const session = activeSession();
      await send('Old turn');
      const oldRequestId =
        engine === 'mother'
          ? vi.mocked(api.sendAgentMessage).mock.lastCall![0].request_id
          : vi.mocked(api.parasiteSendMessage).mock.lastCall![0].requestId;
      expect(oldRequestId).toEqual(expect.any(String));
      act(() => state.abortAgent());
      act(() => {
        vi.advanceTimersByTime(3000);
      });
      await send('New turn');
      expect(state.activeConversationId).toBe(conversationId);
      const newRequestId =
        engine === 'mother'
          ? vi.mocked(api.sendAgentMessage).mock.lastCall![0].request_id
          : vi.mocked(api.parasiteSendMessage).mock.lastCall![0].requestId;
      expect(newRequestId).not.toBe(oldRequestId);
      const emit = engine === 'mother' ? emitAgent : emitParasite;
      emit({
        type: 'text_delta',
        text: 'Old turn reply',
        conversation_id: session,
        request_id: oldRequestId,
      });
      emit({ type: 'done', conversation_id: session, request_id: oldRequestId });
      expect(state.isProcessing).toBe(true);
      expect(state.chatOutput.some((m) => 'text' in m && m.text === 'Old turn reply')).toBe(false);
      emit({ type: 'text_delta', text: 'New turn reply', conversation_id: session });
      expect(state.chatOutput[state.chatOutput.length - 1]).toEqual({
        type: 'assistant',
        text: 'New turn reply',
      });
      emit({ type: 'done', conversation_id: session });
      expect(state.isProcessing).toBe(false);
    }
  );

  it('passes the conversation scope to Connect send, abort, and reset and isolates its events', async () => {
    stored.set('echobird_parasite_agent', 'claudecode');
    await mount();
    const firstKey = activeSession();
    await send('Connect prompt');
    expect(api.parasiteSendMessage).toHaveBeenLastCalledWith({
      agentId: 'claudecode',
      conversationId: firstKey,
      requestId: expect.any(String),
      message: 'Connect prompt',
    });
    emitParasite({ type: 'text_delta', text: 'Ignored unscoped' });
    emitAgent({ type: 'text_delta', text: 'Wrong engine', conversation_id: firstKey });
    emitParasite({ type: 'text_delta', text: 'Connect reply', conversation_id: firstKey });
    expect(state.chatOutput.map((m) => 'text' in m && m.text)).toEqual([
      'Connect prompt',
      'Connect reply',
    ]);
    act(() => state.abortAgent());
    expect(api.parasiteAbort).toHaveBeenCalledExactlyOnceWith('claudecode', firstKey);
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    await act(async () => {
      await state.clearChat();
    });
    expect(api.parasiteReset).toHaveBeenCalledExactlyOnceWith('claudecode', firstKey);
    act(() => state.createConversation());
    const secondKey = activeSession();
    await send('Another Connect prompt');
    emitParasite({ type: 'done', conversation_id: firstKey });
    emitParasite({ type: 'text_delta', text: 'Old Connect reply', conversation_id: firstKey });
    expect(state.isProcessing).toBe(true);
    expect(state.chatOutput).toHaveLength(1);
    emitParasite({ type: 'text_delta', text: 'New Connect reply', conversation_id: secondKey });
    emitParasite({ type: 'done', conversation_id: secondKey });
    expect(state.isProcessing).toBe(false);
    expect(api.sendAgentMessage).not.toHaveBeenCalled();
  });

  it('preserves the native optional Connect scope only for migrated legacy conversations', async () => {
    stored.set('echobird_parasite_agent', 'claudecode');
    stored.set(
      'echobird:chat:agent_local',
      JSON.stringify([{ role: 'user', content: 'Legacy prompt' }])
    );
    await mount();
    expect(state.activeConversationId).toBe('legacy-local');
    expect(activeSession()).toBe('local');
    await send('Continue native session');
    expect(api.parasiteSendMessage).toHaveBeenLastCalledWith({
      agentId: 'claudecode',
      conversationId: undefined,
      requestId: expect.any(String),
      message: 'Continue native session',
    });
    emitParasite({ type: 'text_delta', text: 'Native reply' });
    expect(state.chatOutput[state.chatOutput.length - 1]).toEqual({
      type: 'assistant',
      text: 'Native reply',
    });
    act(() => state.abortAgent());
    expect(api.parasiteAbort).toHaveBeenCalledExactlyOnceWith('claudecode', undefined);
    emitParasite({ type: 'done' });
    await act(async () => {
      await state.clearChat();
    });
    expect(api.parasiteReset).toHaveBeenCalledExactlyOnceWith('claudecode', undefined);
    act(() => state.createConversation());
    const scopedKey = activeSession();
    await send('New independent session');
    expect(api.parasiteSendMessage).toHaveBeenLastCalledWith({
      agentId: 'claudecode',
      conversationId: scopedKey,
      requestId: expect.any(String),
      message: 'New independent session',
    });
    emitParasite({ type: 'text_delta', text: 'Late native reply' });
    expect(state.chatOutput).toHaveLength(1);
    emitParasite({ type: 'text_delta', text: 'Scoped reply', conversation_id: scopedKey });
    expect(state.chatOutput[state.chatOutput.length - 1]).toEqual({
      type: 'assistant',
      text: 'Scoped reply',
    });
  });
});
