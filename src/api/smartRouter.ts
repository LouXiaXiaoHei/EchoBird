import { invoke } from '@tauri-apps/api/core';
import type { ModelConfig } from './types';

export interface SmartRouterConfig {
  enabled: boolean;
  candidateIds: string[];
  disabledCandidateIds?: string[];
  usableCandidateCount: number;
  baseUrl: string;
  modelId: string;
  port: number;
  running: boolean;
}

export interface SmartRouterActivity {
  candidateId: string | null;
  active: boolean;
  activeCandidateIds?: string[];
  sequence: number;
  updatedAtMs: number;
}

export interface SmartRouterProbeResult {
  latencyMs: number | null;
  error: string | null;
}

export async function testSmartRouterCandidate(
  internalId: string,
  modelId: string,
  requestId: string,
  batchId?: string
): Promise<SmartRouterProbeResult> {
  return invoke('test_smart_router_candidate', { internalId, modelId, requestId, batchId });
}

export async function cancelSmartRouterProbe(requestId: string): Promise<void> {
  return invoke('cancel_smart_router_probe', { requestId });
}

export async function getSmartRouterConfig(): Promise<SmartRouterConfig> {
  return invoke('get_smart_router_config');
}

export async function setSmartRouterEnabled(enabled: boolean): Promise<SmartRouterConfig> {
  return invoke('set_smart_router_enabled', { enabled });
}

export async function setSmartRouterCandidateEnabled(
  internalId: string,
  enabled: boolean
): Promise<SmartRouterConfig> {
  return invoke('set_smart_router_candidate_enabled', { internalId, enabled });
}

export async function getSmartRouterActivity(): Promise<SmartRouterActivity> {
  return invoke('get_smart_router_activity');
}

export async function setSmartRouterCandidates(candidateIds: string[]): Promise<SmartRouterConfig> {
  const result = await invoke<SmartRouterConfig>('set_smart_router_candidates', { candidateIds });
  window.dispatchEvent(new Event('models-changed'));
  return result;
}

export async function getSmartRouterCandidates(): Promise<ModelConfig[]> {
  return invoke('get_smart_router_candidates');
}

export async function removeSmartRouterCandidate(internalId: string): Promise<SmartRouterConfig> {
  const result = await invoke<SmartRouterConfig>('remove_smart_router_candidate', { internalId });
  window.dispatchEvent(new Event('models-changed'));
  return result;
}
