import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import {
  agentMemoryDelete,
  agentMemoryList,
  agentMemorySave,
  agentMemoryUpdate,
} from '../../api/agent';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

beforeEach(() => vi.resetAllMocks());

describe('Mother memory IPC', () => {
  it('reads the list without target changes or writes', async () => {
    vi.mocked(invoke).mockResolvedValueOnce([]);
    expect(await agentMemoryList()).toEqual([]);
    expect(invoke).toHaveBeenCalledExactlyOnceWith('agent_memory_list');
  });

  it('sends explicit global and machine scopes with the native camel-case argument name', async () => {
    await agentMemorySave('user', null, 'Use Chinese', 'conversation');
    await agentMemorySave('target', 'remote', 'Linux server', null);
    expect(invoke).toHaveBeenNthCalledWith(1, 'agent_memory_save', {
      scope: 'user',
      target: null,
      text: 'Use Chinese',
      sourceConversationId: 'conversation',
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'agent_memory_save', {
      scope: 'target',
      target: 'remote',
      text: 'Linux server',
      sourceConversationId: null,
    });
  });

  it('updates only text and deletes only the requested ID', async () => {
    await agentMemoryUpdate('memory-id', 'Corrected fact');
    await agentMemoryDelete('memory-id');
    expect(invoke).toHaveBeenNthCalledWith(1, 'agent_memory_update', {
      id: 'memory-id',
      text: 'Corrected fact',
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'agent_memory_delete', { id: 'memory-id' });
  });
});
