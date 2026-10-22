import type { useCursorAccounts } from './useCursorAccounts';
import type { useAntigravityAccounts } from './useAntigravityAccounts';
import type { useDeepSeekAccounts } from './useDeepSeekAccounts';
import type { useGrokAccounts } from './useGrokAccounts';
import type { useWorkBuddyAccounts } from './useWorkBuddyAccounts';
import type { useTraeAccounts } from './useTraeAccounts';
import type { useKiroAccounts } from './useKiroAccounts';
import type { useZCodeAccounts } from './useZCodeAccounts';
import type { useKimiAccounts } from './useKimiAccounts';
import type { useCodeArtsAccounts } from './useCodeArtsAccounts';
import type { useDevinAccounts } from './useDevinAccounts';
import type { useOpenDesignAccounts } from './useOpenDesignAccounts';
import type { useMinimaxAccounts } from './useMinimaxAccounts';
import type { useClineAccounts } from './useClineAccounts';
import type { useQoderAccounts } from './useQoderAccounts';
import type { useDuMateAccounts } from './useDuMateAccounts';
import type { useDoubaoAccounts } from './useDoubaoAccounts';
import type { useQwenWorkAccounts } from './useQwenWorkAccounts';
import type { useClaudeCodeAccounts } from './useClaudeCodeAccounts';
import type { useClaudeDesktopAccounts } from './useClaudeDesktopAccounts';
import type { useCodexAccounts } from './useCodexAccounts';
import type { useKiloAccounts, useCopilotAccounts } from './useClientAccounts';
import { createContext, useContext } from 'react';
import type { ModelConfig, LocalTool } from '../../api/types';
import type { CodexAccount } from '../../api/tauri';

// ===== Context =====

export interface AppManagerContextType {
  smartRouterEnabled: boolean;
  // Internalized state
  selectedTool: string | null;
  setSelectedTool: (id: string | null) => void;
  launchAfterApply: boolean;
  setLaunchAfterApply: (v: boolean) => void;
  isLaunching: boolean;
  agreedConfigPolicy: boolean;
  setAgreedConfigPolicy: (v: boolean) => void;
  toolModelConfig: Record<string, string | null>;
  handleSelectModel: (toolId: string, modelId: string) => void;
  /** Restore the tool's config back to its official vendor endpoint */
  handleRestoreModel: (toolId: string) => Promise<void>;
  accountCardOrder: string[];
  setAccountCardOrder: (order: string[]) => void;
  claudeCodeAccounts: ReturnType<typeof useClaudeCodeAccounts>;
  claudeDesktopAccounts: ReturnType<typeof useClaudeDesktopAccounts>;
  workBuddyAccounts: ReturnType<typeof useWorkBuddyAccounts>;
  kiroAccounts: ReturnType<typeof useKiroAccounts>;
  traeAccounts: ReturnType<typeof useTraeAccounts>;
  traeAccountGroups: Record<'trae' | 'traecn', ReturnType<typeof useTraeAccounts>>;
  workBuddyAccountGroups: Record<
    'workbuddy' | 'workbuddyai',
    ReturnType<typeof useWorkBuddyAccounts>
  >;
  zcodeAccounts: ReturnType<typeof useZCodeAccounts>;
  kimiAccounts: ReturnType<typeof useKimiAccounts>;
  codeArtsAccounts: ReturnType<typeof useCodeArtsAccounts>;
  devinAccounts: ReturnType<typeof useDevinAccounts>;
  openDesignAccounts: ReturnType<typeof useOpenDesignAccounts>;
  minimaxAccounts: ReturnType<typeof useMinimaxAccounts>;
  clineAccounts: ReturnType<typeof useClineAccounts>;
  qoderAccounts: ReturnType<typeof useQoderAccounts>;
  qoderAccountGroups: Record<'qoder' | 'qodercn', ReturnType<typeof useQoderAccounts>>;
  dumateAccounts: ReturnType<typeof useDuMateAccounts>;
  doubaoAccounts: ReturnType<typeof useDoubaoAccounts>;
  doubaoAccountGroups: Record<'doubaowork' | 'dola', ReturnType<typeof useDoubaoAccounts>>;
  qwenWorkAccounts: ReturnType<typeof useQwenWorkAccounts>;
  qwenWorkAccountGroups: Record<'qwenwork' | 'qwenworkcn', ReturnType<typeof useQwenWorkAccounts>>;
  deepSeekAccounts: ReturnType<typeof useDeepSeekAccounts>;
  grokAccounts: ReturnType<typeof useGrokAccounts>;
  kiloAccounts: ReturnType<typeof useKiloAccounts>;
  copilotAccounts: ReturnType<typeof useCopilotAccounts>;
  manusAccounts: ReturnType<typeof useGrokAccounts>;
  cueAccounts: ReturnType<typeof useGrokAccounts>;
  grokBotAccounts: ReturnType<typeof useCursorAccounts>;
  cursorAccounts: ReturnType<typeof useCursorAccounts>;
  antigravityAccounts: ReturnType<typeof useAntigravityAccounts>;
  codexAccounts: CodexAccount[];
  selectedCodexAccountId: string | null;
  setSelectedCodexAccountId: (id: string | null) => void;
  isLoadingCodexAccounts: boolean;
  codexAccountsLoaded: boolean;
  codexAccountsReadFailed: boolean;
  codexAccountsMutationRevision: number;
  isAddingCodexAccount: boolean;
  codexOAuthRemainingSeconds: number;
  refreshingCodexAccountIds: Set<string>;
  codexAuthorizationFailedIds: Set<string>;
  addCodexAccount: () => Promise<void>;
  cancelCodexLogin: () => void;
  refreshCodexAccountQuota: ReturnType<typeof useCodexAccounts>['refresh'];
  deleteCodexAccount: (account: CodexAccount) => Promise<void>;
  selectedToolData: LocalTool | undefined;
  applyError: string | null;
  setApplyError: (v: string | null) => void;
  // Shared props (from App.tsx)
  detectedTools: LocalTool[];
  setDetectedTools: React.Dispatch<React.SetStateAction<LocalTool[]>>;
  isScanning: boolean;
  scanTools: () => Promise<void>;
  userModels: ModelConfig[];
  codexWebSearch: boolean;
  setCodexWebSearch: (v: boolean) => void;
  claudeDesktopWebSearch: boolean;
  claudeCodeWebSearch: boolean;
  setClaudeWebSearch: (tool: 'claudedesktop' | 'claudecode', value: boolean) => void;
  zcodeWebSearch: boolean;
  zcodeImageInput: boolean;
  setZCodeCapability: (capability: 'webSearch' | 'imageInput', value: boolean) => void;
  workBuddyImageInput: boolean;
  setWorkBuddyImageInput: (value: boolean) => void;
  workBuddy1mMode: boolean;
  setWorkBuddy1mMode: (value: boolean) => void;
  mimoImageInput: boolean;
  mimo1mMode: boolean;
  setMimoCapability: (capability: 'imageInput' | 'oneMContext', value: boolean) => void;
  minimaxImageInput: boolean;
  minimax1mMode: boolean;
  setMinimaxCapability: (capability: 'imageInput' | 'oneMContext', value: boolean) => void;
  qoderImageInput: boolean;
  qoder1mMode: boolean;
  setQoderCapability: (capability: 'imageInput' | 'oneMContext', value: boolean) => void;
  dshImageInput: boolean;
  dsh1mMode: boolean;
  setDshCapability: (capability: 'imageInput' | 'oneMContext', value: boolean) => void;
  /** Desktop profile prefer1m. */
  claudeDesktop1mMode: boolean;
  setClaudeDesktop1mMode: (v: boolean) => void;
  /** Claude Code 1M-context toggle. When on,
   *  apply_claudecode appends `[1m]` to the model id (MODEL / OPUS / SONNET / FABLE
   *  env vars only — HAIKU + SUBAGENT stay bare) so Claude Code budgets the
   *  1M window. CC strips the suffix before sending upstream, so the provider
   *  still sees the bare id. */
  claude1mMode: boolean;
  setClaude1mMode: (v: boolean) => void;
  // Launch handler
  handleLaunch: () => Promise<void>;
  // Navigation — internal handler: (toolId, toolName) => fetch install info → call prop
  onGoToMother: (toolId: string, toolName: string) => void;
  // AI-installable tool IDs (from bundled tools/install/index.json)
  aiInstallableIds: string[];
  viewMode: 'desktop' | 'install';
  setViewMode: (mode: 'desktop' | 'install') => void;
}

export const AppManagerContext = createContext<AppManagerContextType | null>(null);

export const useAppManager = () => {
  const ctx = useContext(AppManagerContext);
  if (!ctx) throw new Error('useAppManager must be used within AppManagerProvider');
  return ctx;
};
