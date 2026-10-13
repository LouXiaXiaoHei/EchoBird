// Agent APIs — Mother Agent send/abort/reset/listen
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { AgentRequest, AgentEvent } from './types';

export async function sendAgentMessage(request: AgentRequest): Promise<string> {
  return invoke('agent_send_message', { request });
}

export async function abortAgent(serverKey: string): Promise<boolean> {
  return invoke('agent_abort', { serverKey });
}

export async function resetAgent(serverKey: string): Promise<string> {
  return invoke('agent_reset', { serverKey });
}

export function listenAgentEvents(handler: (event: AgentEvent) => void): Promise<UnlistenFn> {
  return listen<AgentEvent>('agent_event', (e) => handler(e.payload));
}

export interface AgentMemoryRecord {
  id: string;
  text: string;
  scope: 'user' | 'target';
  target: string | null;
  source_conversation_id: string | null;
  created_at: number;
  updated_at: number;
}

export function agentMemoryList(): Promise<AgentMemoryRecord[]> {
  return invoke('agent_memory_list');
}

export function agentMemorySave(
  scope: AgentMemoryRecord['scope'],
  target: string | null,
  text: string,
  sourceConversationId: string | null
): Promise<AgentMemoryRecord> {
  return invoke('agent_memory_save', { scope, target, text, sourceConversationId });
}

export function agentMemoryUpdate(id: string, text: string): Promise<AgentMemoryRecord> {
  return invoke('agent_memory_update', { id, text });
}

export function agentMemoryDelete(id: string): Promise<void> {
  return invoke('agent_memory_delete', { id });
}
