import type { ModelConfig, ReasoningEffort } from '../../api/types';

const REASONING_LEVELS: ReasoningEffort[] = ['low', 'medium', 'high', 'xhigh', 'max'];

export function getReasoningLevels(model?: ModelConfig): ReasoningEffort[] {
  return model ? REASONING_LEVELS : [];
}

export const REASONING_STORAGE_KEY = 'echobird_mother_reasoning';

export function loadReasoningPreferences(): Record<string, ReasoningEffort> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(REASONING_STORAGE_KEY) || '{}');
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(
      Object.entries(value).filter(
        ([, effort]) =>
          typeof effort === 'string' && REASONING_LEVELS.includes(effort as ReasoningEffort)
      )
    );
  } catch {
    return {};
  }
}
