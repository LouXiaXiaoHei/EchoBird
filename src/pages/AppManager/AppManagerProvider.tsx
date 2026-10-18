import { useCursorAccounts } from './useCursorAccounts';
import { useAntigravityAccounts } from './useAntigravityAccounts';
import { useDeepSeekAccounts } from './useDeepSeekAccounts';
import { useGrokAccounts } from './useGrokAccounts';
import { useKiloAccounts, useCopilotAccounts } from './useClientAccounts';
import { accountError } from '../../utils/accountError';
import { ClaudeCodeLoginDialog } from './ClaudeCodeLoginDialog';
import { useWorkBuddyAccounts } from './useWorkBuddyAccounts';
import { useTraeAccounts } from './useTraeAccounts';
import { useKiroAccounts } from './useKiroAccounts';
import { useZCodeAccounts } from './useZCodeAccounts';
import { useKimiAccounts } from './useKimiAccounts';
import { useCodeArtsAccounts } from './useCodeArtsAccounts';
import { switchCodeArtsAccount } from '../../api/codeArtsAccounts';
import { switchKimiAccount } from '../../api/kimiAccounts';
import { useOpenDesignAccounts } from './useOpenDesignAccounts';
import { useMinimaxAccounts } from './useMinimaxAccounts';
import { useClineAccounts } from './useClineAccounts';
import { useQoderAccounts } from './useQoderAccounts';
import { useClaudeCodeAccounts } from './useClaudeCodeAccounts';
import { useClaudeDesktopAccounts } from './useClaudeDesktopAccounts';
import React, { useState, useEffect, useCallback, useRef } from 'react';
import { useCodexAccounts } from './useCodexAccounts';
import { useI18n } from '../../hooks/useI18n';
import * as api from '../../api/tauri';
import type { ModelConfig } from '../../api/types';
import { AppManagerContext } from './context';
import { useToolsStore } from '../../stores/toolsStore';
import { useNavigationStore } from '../../stores/navigationStore';
import { useModelNexus } from '../ModelNexus/context';
import { useFreeModels } from '../FreeModels/provider';
import { getOfficialEndpoint, isOfficialModelSentinel } from '../../data/officialEndpoints';
import { open as openDialog } from '@tauri-apps/plugin-dialog';

// ===== Provider =====

interface AppManagerProviderProps {
  children: React.ReactNode;
}

export const AppManagerProvider: React.FC<AppManagerProviderProps> = ({ children }) => {
  const { t, locale } = useI18n();
  const [accountCardOrder, setAccountCardOrder] = useState<string[]>(() => {
    try {
      const raw = localStorage.getItem('echobird_account_card_order');
      const saved: unknown = raw ? JSON.parse(raw) : [];
      return Array.isArray(saved) ? saved.filter((id): id is string => typeof id === 'string') : [];
    } catch {
      return [];
    }
  });
  const accountOrderIndex = new Map(accountCardOrder.map((id, index) => [id, index]));
  const orderedAccounts = <A extends { id: string }>(provider: string, accounts: A[]) =>
    [...accounts].sort(
      (a, b) =>
        (accountOrderIndex.get(`${provider}:${a.id}`) ?? Infinity) -
        (accountOrderIndex.get(`${provider}:${b.id}`) ?? Infinity)
    );

  // From stores (replaces drilled props)
  const { detectedTools, setDetectedTools, isScanning, scanTools } = useToolsStore();
  const { activePage, goToMother } = useNavigationStore();
  const isActive = activePage === 'apps';

  // Wrapped navigation: build prefill and go to Mother Agent (model check happens there)
  const handleGoToMother = useCallback(
    async (_toolId: string, toolName: string) => {
      const prefill = t('mother.hintInstall').replace('{agent}', toolName);
      goToMother(prefill);
    },
    [t, goToMother]
  );

  const [toolModelConfig, setToolModelConfig] = useState<Record<string, string | null>>({
    claudecode: null,
    openclaw: null,
    opencode: null,
    codex: null,
    hermes: null,
    zcode: null,
  });

  // Load models internally. userModels is consumed by BOTH the AppManager
  // right panel AND the 我的AI项目 right panel (same ModelListSection
  // component), so reload on either activation — otherwise a model added
  // in 模型中心 only surfaces in 我的AI项目 after the user incidentally
  // bounces through 应用桌面 (which flips isActive). Also reload when Model
  // Nexus changes, including edits/deletes from this panel.
  const [userModels, setUserModels] = useState<ModelConfig[]>([]);
  const { userModels: modelNexusModels } = useModelNexus();
  const { routerEnabled, routerBaseUrl } = useFreeModels();
  const userModelsActive = isActive || activePage === 'myProjects';
  useEffect(() => {
    if (!api.getModels) return;
    // Guard against out-of-order resolution: rapid 应用桌面/我的AI项目 toggles can
    // overlap getModels() calls, and a slower earlier one resolving last would
    // clobber userModels with stale data.
    let ignore = false;
    api
      .getModels()
      .then((models) => {
        if (ignore) return;
        setUserModels(models);
        setToolModelConfig((prev) => {
          const removed = Object.keys(prev).filter(
            (toolId) =>
              prev[toolId] &&
              !isOfficialModelSentinel(prev[toolId]!) &&
              !models.some((model) => model.internalId === prev[toolId])
          );
          if (removed.length === 0) return prev;
          const next = { ...prev };
          removed.forEach((toolId) => (next[toolId] = null));
          return next;
        });
      })
      .catch((e) => console.error('Load models failed:', e));
    return () => {
      ignore = true;
    };
  }, [userModelsActive, modelNexusModels, routerEnabled, routerBaseUrl]);

  // AI-installable IDs from bundled install/index.json (offline-first).
  const [aiInstallableIds, setAiInstallableIds] = useState<string[]>([]);
  useEffect(() => {
    if (!isActive) return;
    api
      .getInstallIndex()
      .then((s) => {
        try {
          const data = JSON.parse(s);
          if (Array.isArray(data?.ids)) setAiInstallableIds(data.ids);
        } catch {
          /* malformed — keep empty */
        }
      })
      .catch(() => {
        /* IPC error — keep empty */
      });
  }, [isActive]);

  // Internalized state
  const [selectedTool, setSelectedTool] = useState<string | null>(null);
  const clineSelectionRevision = useRef(0);
  const accountNavigation = useRef({ tool: selectedTool, page: activePage });
  useEffect(() => {
    clineSelectionRevision.current += 1;
    accountNavigation.current = { tool: selectedTool, page: activePage };
  }, [selectedTool, activePage]);
  const accountsEnabled =
    isActive && detectedTools.some((tool) => tool.id === selectedTool && tool.installed);
  const accountGroupEnabled = (...ids: string[]) =>
    activePage === 'accounts'
      ? detectedTools.some((tool) => ids.includes(tool.id) && tool.installed)
      : accountsEnabled && ids.includes(selectedTool ?? '');
  const [isLaunching, setIsLaunching] = useState(false);
  const launchGeneration = useRef(0);
  const [applyError, setApplyError] = useState<string | null>(null);
  const isCodexTool = selectedTool === 'codex' || selectedTool === 'chatgptdesktop';
  const codexManaged = useCodexAccounts(
    accountGroupEnabled('codex', 'chatgptdesktop'),
    !!(selectedTool && toolModelConfig[selectedTool]),
    () => setToolModelConfig((prev) => ({ ...prev, codex: null, chatgptdesktop: null })),
    setApplyError,
    activePage === 'accounts' ? 'accounts' : (selectedTool ?? '')
  );
  const {
    accounts: codexAccounts,
    selectedId: selectedCodexAccountId,
    select: selectCodexAccount,
    loading: isLoadingCodexAccounts,
    loaded: codexAccountsLoaded,
    readFailed: codexAccountsReadFailed,
    mutationRevision: codexAccountsMutationRevision,
    busy: isAddingCodexAccount,
    remainingSeconds: codexOAuthRemainingSeconds,
    refreshing: refreshingCodexAccountIds,
    authorizationFailedIds: codexAuthorizationFailedIds,
    reload: loadCodexAccounts,
    add: addCodexAccount,
    cancelLogin: cancelCodexLogin,
    remove: deleteCodexAccount,
    refresh: refreshCodexAccountQuota,
  } = codexManaged;

  // Bottom-bar checkbox states are persisted across sessions — users get tired of
  // re-checking the same boxes every launch. Default both to true so picking an app
  // and clicking the big button "just launches it"; users who only want to rewrite
  // config without launching can uncheck the launch box.
  const readBool = (key: string, fallback: boolean): boolean => {
    try {
      const v = localStorage.getItem(key);
      return v === null ? fallback : v === 'true';
    } catch {
      return fallback;
    }
  };
  const writeBool = (key: string, v: boolean) => {
    try {
      localStorage.setItem(key, String(v));
    } catch {
      /* private mode */
    }
  };
  const [launchAfterApply, setLaunchAfterApplyRaw] = useState<boolean>(() =>
    readBool('echobird_appmgr_launch_after', true)
  );
  const setLaunchAfterApply = (v: boolean) => {
    setLaunchAfterApplyRaw(v);
    writeBool('echobird_appmgr_launch_after', v);
  };
  const [agreedConfigPolicy, setAgreedConfigPolicyRaw] = useState<boolean>(() =>
    readBool('echobird_appmgr_apply_config', true)
  );
  const setAgreedConfigPolicy = (v: boolean) => {
    setAgreedConfigPolicyRaw(v);
    writeBool('echobird_appmgr_apply_config', v);
  };
  const [codexWebSearch, setCodexWebSearchRaw] = useState<boolean>(() =>
    readBool('echobird_codex_web_search', true)
  );
  const [claudeDesktopWebSearch, setClaudeDesktopWebSearch] = useState<boolean>(() =>
    readBool('echobird_claudedesktop_web_search', true)
  );
  const [claudeCodeWebSearch, setClaudeCodeWebSearch] = useState<boolean>(() =>
    readBool('echobird_claudecode_web_search', true)
  );
  // Claude Code 1M-context toggle. When on,
  // apply_claudecode appends `[1m]` to the model id (MODEL / OPUS / SONNET / FABLE env
  // vars only — HAIKU + SUBAGENT stay bare) so Claude Code budgets the 1M
  // window. CC strips the suffix before sending upstream, so the provider
  // still sees the bare id. Default off: only opt in for an upstream model
  // that supports 1M. Persisted under its own localStorage key
  // (echobird_claudecode_1m_mode) — deliberately not reusing the v5.3.8
  // echobird_claude_1m_mode key, which shared semantics with Claude Desktop.
  const [claude1mMode, setClaude1mModeRaw] = useState<boolean>(() =>
    readBool('echobird_claudecode_1m_mode', false)
  );
  const [claudeDesktop1mMode, setClaudeDesktop1mModeRaw] = useState<boolean>(() =>
    readBool('echobird_claudedesktop_1m_mode', false)
  );
  const [viewMode, setViewModeRaw] = useState<'desktop' | 'install'>('desktop');
  const setViewMode = (mode: 'desktop' | 'install') => {
    if (mode === viewMode) return;
    setViewModeRaw(mode);
    setSelectedTool(null);
    setApplyError(null);
  };
  const clearClaudeCodeModel = useCallback(() => {
    setToolModelConfig((prev) => ({ ...prev, claudecode: null }));
  }, []);
  const claudeCodeAccounts = useClaudeCodeAccounts(
    accountGroupEnabled('claudecode'),
    !!toolModelConfig.claudecode,
    clearClaudeCodeModel,
    setApplyError
  );
  const clearClaudeDesktopModel = useCallback(() => {
    setToolModelConfig((prev) => ({ ...prev, claudedesktop: null }));
  }, []);
  const claudeDesktopAccounts = useClaudeDesktopAccounts(
    accountGroupEnabled('claudedesktop'),
    !!toolModelConfig.claudedesktop,
    clearClaudeDesktopModel,
    setApplyError
  );

  const workBuddyEdition =
    selectedTool === 'workbuddy' || selectedTool === 'workbuddyai' ? selectedTool : null;
  const clearWorkBuddyModel = useCallback((edition: api.WorkBuddyEdition) => {
    setToolModelConfig((prev) => ({ ...prev, [edition]: null }));
  }, []);
  const workBuddyCnAccounts = useWorkBuddyAccounts(
    'workbuddy',
    !!toolModelConfig.workbuddy,
    clearWorkBuddyModel,
    setApplyError,
    accountGroupEnabled('workbuddy')
  );
  const workBuddyAiAccounts = useWorkBuddyAccounts(
    'workbuddyai',
    !!toolModelConfig.workbuddyai,
    clearWorkBuddyModel,
    setApplyError,
    accountGroupEnabled('workbuddyai')
  );
  const workBuddyAccounts =
    workBuddyEdition === 'workbuddyai' ? workBuddyAiAccounts : workBuddyCnAccounts;

  const traeCnAccounts = useTraeAccounts('traecn', accountGroupEnabled('traecn'), setApplyError);
  const traeGlobalAccounts = useTraeAccounts('trae', accountGroupEnabled('trae'), setApplyError);
  const traeAccounts = selectedTool === 'trae' ? traeGlobalAccounts : traeCnAccounts;
  const kiroEdition =
    selectedTool === 'kirocli' && accountGroupEnabled('kirocli')
      ? 'kirocli'
      : accountGroupEnabled('kirocrew')
        ? 'kirocrew'
        : 'kirocli';
  const kiroAccounts = useKiroAccounts(
    kiroEdition,
    accountGroupEnabled('kirocrew', 'kirocli'),
    setApplyError
  );

  const clearMinimaxModel = useCallback(() => {
    setToolModelConfig((prev) => ({ ...prev, minimaxcode: null, minimaxdesktop: null }));
  }, []);
  const minimaxAccounts = useMinimaxAccounts(
    accountGroupEnabled('minimaxcode', 'minimaxdesktop'),
    !!(
      selectedTool &&
      ['minimaxcode', 'minimaxdesktop'].includes(selectedTool) &&
      toolModelConfig[selectedTool]
    ),
    clearMinimaxModel,
    setApplyError,
    activePage === 'accounts' ? 'accounts' : (selectedTool ?? '')
  );
  const qoderEdition: api.QoderEdition = selectedTool === 'qodercn' ? 'qodercn' : 'qoder';
  const clearQoderGlobalModel = useCallback(() => {
    setToolModelConfig((prev) => ({ ...prev, qoder: null }));
  }, []);
  const clearQoderCnModel = useCallback(() => {
    setToolModelConfig((prev) => ({ ...prev, qodercn: null }));
  }, []);
  const qoderGlobalAccounts = useQoderAccounts(
    'qoder',
    accountGroupEnabled('qoder'),
    !!toolModelConfig.qoder,
    clearQoderGlobalModel,
    setApplyError,
    activePage === 'accounts' ? 'accounts' : (selectedTool ?? '')
  );
  const qoderCnAccounts = useQoderAccounts(
    'qodercn',
    accountGroupEnabled('qodercn'),
    !!toolModelConfig.qodercn,
    clearQoderCnModel,
    setApplyError,
    activePage === 'accounts' ? 'accounts' : (selectedTool ?? '')
  );
  const qoderAccounts = qoderEdition === 'qodercn' ? qoderCnAccounts : qoderGlobalAccounts;

  const clearDeepSeekModel = useCallback(() => {
    setToolModelConfig((prev) => ({ ...prev, dsh: null }));
  }, []);
  const deepSeekAccounts = useDeepSeekAccounts(
    accountGroupEnabled('dsh'),
    !!toolModelConfig.dsh,
    clearDeepSeekModel,
    setApplyError
  );
  const clearGrokModel = useCallback(
    () => setToolModelConfig((prev) => ({ ...prev, grok: null })),
    []
  );
  const selectedToolData = detectedTools.find(
    (t) => t.id === selectedTool && (!isActive || t.installed === (viewMode === 'desktop'))
  );
  const grokAccounts = useGrokAccounts(
    accountGroupEnabled('grok'),
    !!toolModelConfig.grok,
    clearGrokModel,
    setApplyError
  );
  const manusAccounts = useGrokAccounts(
    accountGroupEnabled('manus'),
    false,
    () => {},
    setApplyError,
    'manus'
  );
  const cueAccounts = useGrokAccounts(
    accountGroupEnabled('cue'),
    false,
    () => {},
    setApplyError,
    'cue'
  );

  const clearClineModel = useCallback(() => {
    clineSelectionRevision.current += 1;
    setToolModelConfig((prev) => ({ ...prev, cline: null, clinedesktop: null }));
  }, []);
  const clineAccounts = useClineAccounts(
    accountGroupEnabled('cline', 'clinedesktop'),
    !!(
      selectedTool &&
      ['cline', 'clinedesktop'].includes(selectedTool) &&
      toolModelConfig[selectedTool]
    ),
    clearClineModel,
    setApplyError,
    activePage === 'accounts' ? 'accounts' : (selectedTool ?? '')
  );

  const clearKiloModel = useCallback(
    () => setToolModelConfig((prev) => ({ ...prev, kilo: null, kilodesktop: null })),
    []
  );
  const kiloAccounts = useKiloAccounts(
    accountGroupEnabled('kilo', 'kilodesktop'),
    !!(
      selectedTool &&
      ['kilo', 'kilodesktop'].includes(selectedTool) &&
      toolModelConfig[selectedTool]
    ),
    clearKiloModel,
    setApplyError,
    activePage === 'accounts' ? 'accounts' : (selectedTool ?? '')
  );
  const clearCopilotModel = useCallback(
    () => setToolModelConfig((prev) => ({ ...prev, copilot: null, copilotdesktop: null })),
    []
  );
  const copilotAccounts = useCopilotAccounts(
    accountGroupEnabled('copilot', 'copilotdesktop'),
    !!(
      selectedTool &&
      ['copilot', 'copilotdesktop'].includes(selectedTool) &&
      toolModelConfig[selectedTool]
    ),
    clearCopilotModel,
    setApplyError,
    selectedTool === 'copilotdesktop' ||
      (activePage === 'accounts' &&
        !detectedTools.some((tool) => tool.id === 'copilot' && tool.installed))
      ? 'copilotdesktop'
      : 'copilot',
    activePage === 'accounts' ? 'accounts' : (selectedTool ?? '')
  );

  const clearGrokBotModel = useCallback(
    () => setToolModelConfig((prev) => ({ ...prev, grokbot: null })),
    []
  );
  const grokBotAccounts = useCursorAccounts(
    'grokbot',
    accountGroupEnabled('grokbot'),
    clearGrokBotModel,
    setApplyError
  );

  const clearCursorModel = useCallback(
    () => setToolModelConfig((prev) => ({ ...prev, cursor: null })),
    []
  );
  const cursorAccounts = useCursorAccounts(
    'cursor',
    accountGroupEnabled('cursor'),
    clearCursorModel,
    setApplyError
  );
  const antigravityAccounts = useAntigravityAccounts(
    accountGroupEnabled('antigravity', 'antigravitydesktop'),
    activePage === 'accounts'
      ? 'antigravitydesktop'
      : selectedTool === 'antigravity'
        ? 'antigravity'
        : 'antigravitydesktop',
    setApplyError
  );

  const kimiTool = selectedTool === 'kimicode' ? 'kimicode' : 'kimidesktop';
  const kimiModel = toolModelConfig[kimiTool];
  const kimiSelectionRevision = useRef(0);
  const kimiAccounts = useKimiAccounts(
    accountGroupEnabled('kimicode', 'kimidesktop'),
    !!kimiModel,
    () => setToolModelConfig((prev) => ({ ...prev, [kimiTool]: null })),
    setApplyError,
    activePage === 'accounts' ? 'kimi' : kimiTool
  );
  useEffect(() => {
    kimiSelectionRevision.current += 1;
  }, [selectedTool, activePage, kimiAccounts.selectedId, kimiModel]);
  const zcodeAccounts = useZCodeAccounts(
    accountGroupEnabled('zcode'),
    !!toolModelConfig.zcode,
    () => setToolModelConfig((prev) => ({ ...prev, zcode: null })),
    setApplyError
  );
  const codeArtsAccounts = useCodeArtsAccounts(
    accountGroupEnabled('codearts') ||
      (activePage === 'freeModels' &&
        detectedTools.some((tool) => tool.id === 'codearts' && tool.installed)),
    !!toolModelConfig.codearts,
    () => setToolModelConfig((prev) => ({ ...prev, codearts: null })),
    setApplyError,
    `${activePage}:${selectedTool ?? ''}`
  );
  const codeArtsModel = toolModelConfig.codearts;
  const codeArtsSelectionRevision = useRef(0);
  useEffect(() => {
    codeArtsSelectionRevision.current += 1;
  }, [activePage, selectedTool, codeArtsAccounts.selectedId, codeArtsModel]);
  const openDesignAccounts = useOpenDesignAccounts(
    accountGroupEnabled('opendesign'),
    !!toolModelConfig.opendesign,
    () => setToolModelConfig((prev) => ({ ...prev, opendesign: null })),
    setApplyError
  );

  // Set tool model (single selection) - UI state update
  const handleSelectModel = (toolId: string, modelId: string) => {
    if (toolId === 'codearts') {
      codeArtsSelectionRevision.current += 1;
      codeArtsAccounts.select(null);
    }
    if (toolId === 'cline' || toolId === 'clinedesktop') {
      clineSelectionRevision.current += 1;
      clineAccounts.select(null);
    }
    if (toolId === 'minimaxcode' || toolId === 'minimaxdesktop') minimaxAccounts.select(null);
    if (toolId === 'qoder' || toolId === 'qodercn') qoderAccounts.select(null);
    if (toolId === 'zcode') zcodeAccounts.select(null);
    if (toolId === 'kimicode' || toolId === 'kimidesktop') kimiAccounts.select(null);
    if (toolId === 'opendesign') openDesignAccounts.select(null);
    if (toolId === 'cursor') cursorAccounts.select(null);
    if (toolId === 'grokbot') grokBotAccounts.select(null);
    if (toolId === 'dsh') deepSeekAccounts.select(null);
    if (toolId === 'grok') grokAccounts.select(null);
    if (toolId === 'manus') manusAccounts.select(null);
    if (toolId === 'cue') cueAccounts.select(null);
    if (toolId === 'kilo' || toolId === 'kilodesktop') kiloAccounts.select(null);
    if (toolId === 'copilot' || toolId === 'copilotdesktop') copilotAccounts.select(null);
    if (toolId === 'claudecode') claudeCodeAccounts.setSelectedId(null);
    if (toolId === 'claudedesktop') claudeDesktopAccounts.select(null);
    if (toolId === 'workbuddy' || toolId === 'workbuddyai') workBuddyAccounts.select(null);
    if (toolId === 'codex' || toolId === 'chatgptdesktop') {
      codexManaged.select(null);
    }
    setToolModelConfig((prev) => ({
      ...prev,
      [toolId]: modelId,
    }));
  };

  // Apply model config to backend (internalized from App.tsx).
  const applyModelConfig = async (
    toolId: string,
    internalId: string,
    oneMOverride?: boolean,
    webSearchOverride?: boolean
  ): Promise<true | string | false> => {
    const model = userModels.find((m) => m.internalId === internalId);
    if (!model) {
      console.error('Model not found:', internalId);
      return false;
    }

    const toolData = detectedTools.find((t) => t.id === toolId);
    const toolProtocols = toolData?.apiProtocol || ['openai'];

    // Default to the protocol the model's URL actually speaks — NOT toolProtocols[0].
    // For a single-URL model, defaulting to toolProtocols[0] would
    // write an OpenAI-only model as @ai-sdk/anthropic against an OpenAI URL (or an
    // Anthropic-only model as openai-compatible), 404-ing every call at runtime.
    // Only a both-URL model falls back to toolProtocols[0].
    const modelHasBoth = !!(model.baseUrl && model.anthropicUrl);
    const defaultProtocol = modelHasBoth
      ? toolProtocols[0] === 'anthropic'
        ? 'anthropic'
        : 'openai'
      : model.anthropicUrl
        ? 'anthropic'
        : 'openai';
    const selectedProtocol = defaultProtocol;

    const useAnthropicUrl = selectedProtocol === 'anthropic' && model.anthropicUrl;
    const apiUrl = useAnthropicUrl ? model.anthropicUrl! : model.baseUrl;

    // eslint-disable-next-line no-console
    console.debug(
      `[AppManager] Applying model to ${toolId}: protocol=${selectedProtocol}, url=${apiUrl}`
    );

    // Claude apps connect directly to the selected Anthropic endpoint.
    const isClaudeDesktopApp = toolId === 'claudedesktop';
    const isClaudeCodeApp = toolId === 'claudecode';
    const isClaudeApp = isClaudeDesktopApp || isClaudeCodeApp;
    const effective1m = isClaudeDesktopApp
      ? (oneMOverride ?? claudeDesktop1mMode)
      : isClaudeCodeApp && (oneMOverride ?? claude1mMode);

    try {
      const result = await api.applyModelToTool(toolId, {
        id: model.internalId,
        name: model.name,
        baseUrl: apiUrl,
        apiKey: model.apiKey,
        model: model.modelId || '',
        protocol: selectedProtocol,
        ...(isClaudeApp
          ? {
              oneMContext: effective1m,
              webSearch:
                webSearchOverride ??
                (isClaudeDesktopApp ? claudeDesktopWebSearch : claudeCodeWebSearch),
            }
          : {}),
        ...(['codex', 'chatgptdesktop'].includes(toolId)
          ? {
              webSearch:
                model.internalId === 'smart-router' ? false : (webSearchOverride ?? codexWebSearch),
            }
          : {}),
      });

      if (result?.success) {
        const sharedTools = ['cline', 'clinedesktop'].includes(toolId)
          ? ['cline', 'clinedesktop']
          : ['minimaxcode', 'minimaxdesktop'];
        setDetectedTools((prev) =>
          prev.map((t) =>
            t.id === toolId || (sharedTools.includes(toolId) && sharedTools.includes(t.id))
              ? { ...t, activeModel: model.modelId || model.internalId }
              : t
          )
        );
        if (sharedTools.includes(toolId)) {
          setToolModelConfig((prev) => ({
            ...prev,
            ...Object.fromEntries(sharedTools.map((id) => [id, model.internalId])),
          }));
        }
        return true;
      } else {
        console.error('[AppManager] Failed to apply model:', result?.message);
        const message = result?.message;
        return message?.startsWith('accountError.') ? accountError(message, t) : message || false;
      }
    } catch (error) {
      console.error('[AppManager] Error applying model to tool:', error);
      return false;
    }
  };

  // Restore the shared Web Search preference and apply it on an explicit flip.
  const setCodexWebSearch = (value: boolean) => {
    if (isLaunching || (selectedTool && toolModelConfig[selectedTool] === 'smart-router')) return;
    setCodexWebSearchRaw(value);
    writeBool('echobird_codex_web_search', value);
    if (!isCodexTool || !selectedTool || !selectedToolData?.installed || selectedCodexAccountId)
      return;
    const modelId = toolModelConfig[selectedTool];
    if (!modelId || isOfficialModelSentinel(modelId)) return;
    setIsLaunching(true);
    setApplyError(null);
    void applyModelConfig(selectedTool, modelId, undefined, value)
      .then((result) => {
        if (result !== true)
          setApplyError(typeof result === 'string' ? result : t('key.destroyed'));
      })
      .finally(() => setIsLaunching(false));
  };

  const setClaudeWebSearch = (tool: 'claudedesktop' | 'claudecode', value: boolean) => {
    if (isLaunching || selectedTool !== tool) return;
    const desktop = tool === 'claudedesktop';
    (desktop ? setClaudeDesktopWebSearch : setClaudeCodeWebSearch)(value);
    writeBool(`echobird_${tool}_web_search`, value);
    if (
      !selectedToolData?.installed ||
      (desktop ? claudeDesktopAccounts.selectedId : claudeCodeAccounts.selectedId)
    )
      return;
    const modelId = toolModelConfig[tool];
    if (!modelId || isOfficialModelSentinel(modelId)) return;
    setIsLaunching(true);
    setApplyError(null);
    void applyModelConfig(tool, modelId, undefined, value)
      .then((result) => {
        if (result !== true)
          setApplyError(typeof result === 'string' ? result : t('key.destroyed'));
      })
      .finally(() => setIsLaunching(false));
  };

  const setClaudeOneM = (tool: 'claudedesktop' | 'claudecode', value: boolean) => {
    if (isLaunching || selectedTool !== tool) return;
    const desktop = tool === 'claudedesktop';
    (desktop ? setClaudeDesktop1mModeRaw : setClaude1mModeRaw)(value);
    writeBool(`echobird_${tool}_1m_mode`, value);
    if (
      !selectedToolData?.installed ||
      (desktop ? claudeDesktopAccounts.selectedId : claudeCodeAccounts.selectedId)
    )
      return;
    const modelId = toolModelConfig[tool];
    if (!modelId || isOfficialModelSentinel(modelId)) return;
    setIsLaunching(true);
    setApplyError(null);
    void applyModelConfig(tool, modelId, value)
      .then((result) => {
        if (result !== true)
          setApplyError(typeof result === 'string' ? result : t('key.destroyed'));
      })
      .finally(() => setIsLaunching(false));
  };
  const setClaude1mMode = (value: boolean) => setClaudeOneM('claudecode', value);
  const setClaudeDesktop1mMode = (value: boolean) => setClaudeOneM('claudedesktop', value);

  // Restore = delete the tool's config file. The tool itself regenerates
  // a vendor-default config on next launch, so restore is symmetric with
  // a fresh install. Backend also clears the ~/.echobird/{tool}.json relay
  // for "custom" tools.
  const applyRestore = async (toolId: string): Promise<true | string | false> => {
    try {
      const result = await api.restoreToolToOfficial(toolId);
      if (result?.success) {
        const official = getOfficialEndpoint(toolId);
        setDetectedTools((prev) =>
          prev.map((t) => (t.id === toolId ? { ...t, activeModel: official?.name || '' } : t))
        );
        return true;
      }
      return result?.message || false;
    } catch (err) {
      console.error('[AppManager] Restore-to-official failed:', err);
      return String(err);
    }
  };

  // Direct restore — kept exported on context for any callers that want to
  // bypass the bottom-bar flow. The card click now selects (no immediate
  // apply); the actual restore runs from handleLaunch when the official
  // sentinel is the pending selection.
  const handleRestoreModel = async (toolId: string) => {
    const result = await applyRestore(toolId);
    if (result !== true) {
      setApplyError(typeof result === 'string' ? result : t('key.destroyed'));
    }
  };

  // Launch handler
  const handleLaunch = async () => {
    if (!selectedTool || isLaunching) return;
    if (selectedTool === 'codearts' && codeArtsAccounts.busy) return;
    if (selectedTool === 'zcode' && zcodeAccounts.busy) return;
    if (['kimicode', 'kimidesktop'].includes(selectedTool) && kimiAccounts.busy) return;
    if (selectedTool === 'opendesign' && openDesignAccounts.busy) return;
    if (selectedTool === 'claudedesktop' && claudeDesktopAccounts.busy) return;
    if (selectedTool === 'cue' && cueAccounts.busy) return;
    if (['cline', 'clinedesktop'].includes(selectedTool) && clineAccounts.busy) return;
    if (['kilo', 'kilodesktop'].includes(selectedTool) && kiloAccounts.busy) return;
    if (['copilot', 'copilotdesktop'].includes(selectedTool) && copilotAccounts.busy) return;
    if ((selectedTool === 'trae' || selectedTool === 'traecn') && traeAccounts.busy) return;
    if (['minimaxcode', 'minimaxdesktop'].includes(selectedTool) && minimaxAccounts.busy) return;
    if (['qoder', 'qodercn'].includes(selectedTool) && qoderAccounts.busy) return;
    if ((selectedTool === 'kirocrew' || selectedTool === 'kirocli') && kiroAccounts.busy) return;
    const generation = ++launchGeneration.current;
    const finishLaunch = () => {
      if (launchGeneration.current === generation) setIsLaunching(false);
    };
    const startCooldown = () => {
      setTimeout(finishLaunch, 3000);
    };
    setIsLaunching(true);
    const switchingClaudeAccount =
      (selectedTool === 'claudecode' && !!claudeCodeAccounts.selectedId) ||
      (selectedTool === 'claudedesktop' && !!claudeDesktopAccounts.selectedId);
    if (
      selectedTool !== 'vscode' &&
      selectedTool !== 'copilotdesktop' &&
      !switchingClaudeAccount &&
      !(workBuddyEdition && workBuddyAccounts.selectedId) &&
      !((selectedTool === 'trae' || selectedTool === 'traecn') && traeAccounts.selectedId) &&
      !(['minimaxcode', 'minimaxdesktop'].includes(selectedTool) && minimaxAccounts.selectedId) &&
      !(['qoder', 'qodercn'].includes(selectedTool) && qoderAccounts.selectedId) &&
      !((selectedTool === 'kirocrew' || selectedTool === 'kirocli') && kiroAccounts.selectedId) &&
      !(selectedTool === 'zcode' && zcodeAccounts.selectedId) &&
      !(selectedTool === 'codearts' && codeArtsAccounts.selectedId) &&
      !(['kimicode', 'kimidesktop'].includes(selectedTool) && kimiAccounts.selectedId) &&
      !(selectedTool === 'opendesign' && openDesignAccounts.selectedId) &&
      !(selectedTool === 'dsh' && deepSeekAccounts.selectedId) &&
      !(selectedTool === 'grok' && grokAccounts.selectedId) &&
      !(['cline', 'clinedesktop'].includes(selectedTool) && clineAccounts.selectedId) &&
      !(['kilo', 'kilodesktop'].includes(selectedTool) && kiloAccounts.selectedId) &&
      !(['copilot', 'copilotdesktop'].includes(selectedTool) && copilotAccounts.selectedId) &&
      !(selectedTool === 'manus' && manusAccounts.selectedId) &&
      !(selectedTool === 'cue' && cueAccounts.selectedId) &&
      !(selectedTool === 'cursor' && cursorAccounts.selectedId) &&
      !(selectedTool === 'grokbot' && grokBotAccounts.selectedId) &&
      !(
        (selectedTool === 'antigravity' || selectedTool === 'antigravitydesktop') &&
        antigravityAccounts.selectedId
      )
    )
      startCooldown();

    const toolData = detectedTools.find((t) => t.id === selectedTool);
    const isLaunchable = !!toolData?.launchFile;
    const noModelConfig = !!toolData?.noModelConfig;
    const pickWorkingDirectory = async () => {
      if (toolData?.category !== 'CLI Code' || toolData.noCwdPrompt) return undefined;
      const picked = await openDialog({ directory: true, multiple: false });
      return typeof picked === 'string' && picked ? picked : null;
    };

    // An OpenAI account and a third-party API model are one exclusive choice.
    // Selecting an account restores the official provider before writing that
    // account's auth snapshot; selecting a model clears the account choice.
    if (
      (selectedTool === 'antigravity' || selectedTool === 'antigravitydesktop') &&
      antigravityAccounts.selectedId
    ) {
      try {
        await api.switchAntigravityAccount(antigravityAccounts.selectedId);
        await antigravityAccounts.reload();
      } catch (error) {
        setApplyError(accountError(error, t));
        finishLaunch();
        return;
      }
      startCooldown();
    } else if (isCodexTool && selectedCodexAccountId) {
      const restoreResult = await applyRestore(selectedTool);
      if (restoreResult !== true) {
        setApplyError(
          typeof restoreResult === 'string' ? accountError(restoreResult, t) : t('key.destroyed')
        );
        finishLaunch();
        return;
      }
      try {
        await api.switchCodexAccount(selectedCodexAccountId);
        await loadCodexAccounts();
      } catch (error) {
        setApplyError(accountError(error, t));
        finishLaunch();
        return;
      }
    } else if (
      ((selectedTool === 'kilo' || selectedTool === 'kilodesktop') && kiloAccounts.selectedId) ||
      ((selectedTool === 'copilot' || selectedTool === 'copilotdesktop') &&
        copilotAccounts.selectedId)
    ) {
      try {
        if (selectedTool === 'kilo' || selectedTool === 'kilodesktop') {
          await api.switchKiloAccount(kiloAccounts.selectedId!, selectedTool);
          await kiloAccounts.reload();
        } else {
          await api.switchCopilotAccount(
            copilotAccounts.selectedId!,
            selectedTool as api.CopilotTool
          );
          await copilotAccounts.reload();
        }
        setDetectedTools((prev) =>
          prev.map((tool) => (tool.id === selectedTool ? { ...tool, activeModel: '' } : tool))
        );
        if (launchAfterApply) {
          let cwd: string | undefined;
          if (toolData?.category === 'CLI Code' && !toolData.noCwdPrompt) {
            const picked = await openDialog({ directory: true, multiple: false });
            if (typeof picked !== 'string' || !picked) return;
            cwd = picked;
          }
          await api.startTool(selectedTool, toolData?.startCommand, cwd);
        }
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (selectedTool === 'dsh' && deepSeekAccounts.selectedId) {
      try {
        await api.switchDeepSeekAccount(deepSeekAccounts.selectedId, locale);
        setDetectedTools((prev) =>
          prev.map((tool) => (tool.id === 'dsh' ? { ...tool, activeModel: '' } : tool))
        );
        await deepSeekAccounts.reload();
        if (launchAfterApply) await api.startTool('dsh');
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (selectedTool === 'grokbot' && grokBotAccounts.selectedId) {
      try {
        await api.switchGrokBotAccount(grokBotAccounts.selectedId);
        await grokBotAccounts.reload();
        await api.startTool('grokbot');
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (selectedTool === 'cursor' && cursorAccounts.selectedId) {
      try {
        await api.switchCursorAccount(cursorAccounts.selectedId);
        await cursorAccounts.reload();
        await api.startTool('cursor');
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (
      (selectedTool === 'manus' && manusAccounts.selectedId) ||
      (selectedTool === 'cue' && cueAccounts.selectedId)
    ) {
      try {
        await (selectedTool === 'cue' ? cueAccounts : manusAccounts).switchAccount();
        await api.startTool(selectedTool);
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (selectedTool === 'grok' && grokAccounts.selectedId) {
      try {
        const restored = await applyRestore('grok');
        if (restored !== true)
          throw new Error(typeof restored === 'string' ? restored : t('key.destroyed'));
        await grokAccounts.switchAccount();
        startCooldown();
      } catch (error) {
        setApplyError(accountError(error, t));
        finishLaunch();
        return;
      }
    } else if (['cline', 'clinedesktop'].includes(selectedTool) && clineAccounts.selectedId) {
      const selectionRevision = clineSelectionRevision.current;
      const current = () =>
        selectionRevision === clineSelectionRevision.current &&
        accountNavigation.current.tool === selectedTool &&
        accountNavigation.current.page === activePage;
      try {
        const cwd = launchAfterApply ? await pickWorkingDirectory() : undefined;
        if (cwd === null || !current()) return;
        await api.switchClineAccount(clineAccounts.selectedId, selectedTool as api.ClineTool);
        setDetectedTools((prev) =>
          prev.map((tool) =>
            ['cline', 'clinedesktop'].includes(tool.id) ? { ...tool, activeModel: '' } : tool
          )
        );
        if (selectionRevision === clineSelectionRevision.current)
          setToolModelConfig((prev) => ({ ...prev, cline: null, clinedesktop: null }));
        await clineAccounts.reload();
        if (launchAfterApply && current())
          await api.startTool(selectedTool, toolData?.startCommand, cwd);
      } catch (error) {
        if (current()) setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (
      ['minimaxcode', 'minimaxdesktop'].includes(selectedTool) &&
      minimaxAccounts.selectedId
    ) {
      try {
        const cwd = launchAfterApply ? await pickWorkingDirectory() : undefined;
        if (cwd === null) return;
        await api.switchMinimaxAccount(minimaxAccounts.selectedId, selectedTool as api.MinimaxTool);
        setDetectedTools((prev) =>
          prev.map((tool) =>
            ['minimaxcode', 'minimaxdesktop'].includes(tool.id)
              ? { ...tool, activeModel: '' }
              : tool
          )
        );
        await minimaxAccounts.reload();
        if (launchAfterApply) await api.startTool(selectedTool, toolData?.startCommand, cwd);
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (['qoder', 'qodercn'].includes(selectedTool) && qoderAccounts.selectedId) {
      try {
        await api.switchQoderAccount(
          qoderEdition,
          qoderAccounts.selectedId,
          selectedTool as api.QoderTool
        );
        await qoderAccounts.reload();
        await api.startTool(selectedTool, toolData?.startCommand);
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (selectedTool === 'codearts' && codeArtsAccounts.selectedId) {
      const selectionRevision = codeArtsSelectionRevision.current;
      const current = () =>
        selectionRevision === codeArtsSelectionRevision.current &&
        accountNavigation.current.tool === selectedTool &&
        accountNavigation.current.page === activePage;
      try {
        const cwd = launchAfterApply ? await pickWorkingDirectory() : undefined;
        if (cwd === null || !current()) return;
        await switchCodeArtsAccount(codeArtsAccounts.selectedId);
        if (current()) {
          await codeArtsAccounts.reload();
        }
        if (launchAfterApply && current())
          await api.startTool(selectedTool, toolData?.startCommand, cwd);
      } catch (error) {
        if (current()) setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (selectedTool === 'opendesign' && openDesignAccounts.selectedId) {
      try {
        await api.switchOpenDesignAccount(openDesignAccounts.selectedId);
        setDetectedTools((prev) =>
          prev.map((tool) => (tool.id === 'opendesign' ? { ...tool, activeModel: '' } : tool))
        );
        await openDesignAccounts.reload();
        if (launchAfterApply) await api.startTool('opendesign');
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (['kimicode', 'kimidesktop'].includes(selectedTool) && kimiAccounts.selectedId) {
      const selectionRevision = kimiSelectionRevision.current;
      const current = () =>
        selectionRevision === kimiSelectionRevision.current &&
        accountNavigation.current.tool === selectedTool &&
        accountNavigation.current.page === activePage;
      try {
        const cwd = launchAfterApply ? await pickWorkingDirectory() : undefined;
        if (cwd === null || !current()) return;
        await switchKimiAccount(kimiAccounts.selectedId);
        setDetectedTools((prev) =>
          prev.map((tool) =>
            ['kimicode', 'kimidesktop'].includes(tool.id) ? { ...tool, activeModel: '' } : tool
          )
        );
        if (current()) {
          setToolModelConfig((prev) => ({ ...prev, kimicode: null, kimidesktop: null }));
          await kimiAccounts.reload();
        }
        if (launchAfterApply && current())
          await api.startTool(selectedTool, toolData?.startCommand, cwd);
      } catch (error) {
        if (current()) setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (selectedTool === 'zcode' && zcodeAccounts.selectedId) {
      try {
        await api.switchZCodeAccount(zcodeAccounts.selectedId);
        setDetectedTools((prev) =>
          prev.map((tool) => (tool.id === 'zcode' ? { ...tool, activeModel: '' } : tool))
        );
        await zcodeAccounts.reload();
        if (launchAfterApply) await api.startTool('zcode', toolData?.startCommand);
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (workBuddyEdition && workBuddyAccounts.selectedId) {
      try {
        await api.switchWorkBuddyAccount(workBuddyEdition, workBuddyAccounts.selectedId);
        await workBuddyAccounts.reload();
        if (launchAfterApply) await api.startTool(workBuddyEdition, toolData?.startCommand);
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (
      (selectedTool === 'kirocrew' || selectedTool === 'kirocli') &&
      kiroAccounts.selectedId
    ) {
      try {
        await api.switchKiroAccount(selectedTool, kiroAccounts.selectedId);
        await kiroAccounts.reload();
      } catch (error) {
        setApplyError(accountError(error, t));
        finishLaunch();
        return;
      }
    } else if ((selectedTool === 'trae' || selectedTool === 'traecn') && traeAccounts.selectedId) {
      try {
        await api.switchTraeAccount(selectedTool, traeAccounts.selectedId);
        await traeAccounts.reload();
        if (launchAfterApply || noModelConfig)
          await api.startTool(selectedTool, toolData?.startCommand);
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (selectedTool === 'claudedesktop' && claudeDesktopAccounts.selectedId) {
      try {
        await api.switchClaudeDesktopAccount(claudeDesktopAccounts.selectedId);
        const restored = await applyRestore(selectedTool);
        if (restored !== true)
          throw new Error(typeof restored === 'string' ? restored : t('key.destroyed'));
        await claudeDesktopAccounts.reload();
        if (launchAfterApply) await api.startTool(selectedTool, toolData?.startCommand);
      } catch (error) {
        setApplyError(accountError(error, t));
      } finally {
        finishLaunch();
      }
      return;
    } else if (selectedTool === 'claudecode' && claudeCodeAccounts.selectedId) {
      try {
        const restored = await applyRestore(selectedTool);
        if (restored !== true)
          throw new Error(typeof restored === 'string' ? restored : t('key.destroyed'));
        await api.switchClaudeCodeAccount(claudeCodeAccounts.selectedId);
        await claudeCodeAccounts.reload();
        startCooldown();
      } catch (error) {
        setApplyError(accountError(error, t));
        finishLaunch();
        return;
      }
    } else if (
      !noModelConfig &&
      agreedConfigPolicy &&
      !isLaunchable &&
      toolModelConfig[selectedTool]
    ) {
      const pending = toolModelConfig[selectedTool]!;
      const applyResult = isOfficialModelSentinel(pending)
        ? await applyRestore(selectedTool)
        : await applyModelConfig(selectedTool, pending);
      if (applyResult !== true) {
        setApplyError(typeof applyResult === 'string' ? applyResult : t('key.destroyed'));
        finishLaunch();
        return;
      }
    }
    // Launch tool when "launch directly" is checked, or unconditionally for desktop apps
    if (
      launchAfterApply ||
      noModelConfig ||
      ['qoder', 'qodercn', 'claudescience'].includes(selectedTool)
    ) {
      if (isLaunchable) {
        // Launchable tool (e.g. game): open independent window with model config
        const selectedModelId = toolModelConfig[selectedTool];
        const selectedModel = selectedModelId
          ? userModels.find((m) => m.internalId === selectedModelId)
          : undefined;
        const modelConfig = selectedModel
          ? {
              baseUrl: selectedModel.baseUrl,
              anthropicUrl: selectedModel.anthropicUrl,
              apiKey: selectedModel.apiKey,
              model: selectedModel.modelId || selectedModel.name || 'unknown',
              name: selectedModel.name,
              protocol: selectedModel.baseUrl ? 'openai' : 'anthropic',
              locale,
            }
          : { locale };
        const result = await api.launchGame(selectedTool, toolData!.launchFile!, modelConfig);
        if (result && !result.success) {
          console.error('Failed to launch:', result.message);
          if (result.message) setApplyError(result.message);
        } else if (selectedModel) {
          // Mirror the apply-path optimistic update (see line ~209). launchable
          // tools (games / WebView utilities) inject the model via
          // window.__MODEL_CONFIG__ instead of writing a config file, so the
          // apply path is skipped — without this, the tool card would forever
          // show "模型: -" even after a successful launch.
          setDetectedTools((prev) =>
            prev.map((t) =>
              t.id === selectedTool
                ? { ...t, activeModel: selectedModel.modelId || selectedModel.internalId }
                : t
            )
          );
        }
      } else {
        // Only "CLI Code" tools prompt for a working folder — their launch goes
        // through start_cli_tool, which honors cwd. Desktop / IDE / Utility /
        // Game tools launch via start_gui_tool / launch_vscode / launchGame,
        // none of which consume cwd, so prompting would silently discard the
        // user's pick (Coffee-CLI-style launchpad, minus persistence — we
        // re-prompt every launch). If the user cancels the picker, abort the
        // launch entirely — don't fall back to home, which is the bug this
        // fixes (CLI tools used to spawn in ~/ and were useless for development).
        // A CLI tool may opt out per-tool via paths.json `noCwdPrompt: true`
        // (e.g. openclaw) — it launches directly with no prompt, spawning in
        // the home dir, which is fine for self-contained CLI agents.
        let cwd: string | undefined;
        try {
          const picked = await pickWorkingDirectory();
          if (picked === null) {
            finishLaunch();
            return;
          }
          cwd = picked;
        } catch (e) {
          console.error('[AppManager] folder picker failed:', e);
          setApplyError(e instanceof Error ? e.message : String(e));
          finishLaunch();
          return;
        }
        try {
          await api.startTool(selectedTool, toolData?.startCommand, cwd);
        } catch (err) {
          console.error('Failed to launch tool:', err);
          setApplyError(err instanceof Error ? err.message : String(err));
        } finally {
          if (
            (selectedTool === 'kirocrew' || selectedTool === 'kirocli') &&
            kiroAccounts.selectedId
          )
            finishLaunch();
        }
      }
    }
    if (selectedTool === 'vscode' || selectedTool === 'copilotdesktop') startCooldown();
  };

  return (
    <AppManagerContext.Provider
      value={{
        smartRouterEnabled: routerEnabled,
        selectedTool,
        setSelectedTool,
        launchAfterApply,
        setLaunchAfterApply,
        isLaunching,
        agreedConfigPolicy,
        setAgreedConfigPolicy,
        toolModelConfig,
        handleSelectModel,
        handleRestoreModel,
        accountCardOrder,
        setAccountCardOrder,
        claudeCodeAccounts: {
          ...claudeCodeAccounts,
          accounts: orderedAccounts('claudecode', claudeCodeAccounts.accounts),
        },
        claudeDesktopAccounts: {
          ...claudeDesktopAccounts,
          accounts: orderedAccounts('claudedesktop', claudeDesktopAccounts.accounts),
        },
        workBuddyAccounts: {
          ...workBuddyAccounts,
          accounts: orderedAccounts(workBuddyEdition ?? 'workbuddy', workBuddyAccounts.accounts),
        },
        workBuddyAccountGroups: {
          workbuddy: {
            ...workBuddyCnAccounts,
            accounts: orderedAccounts('workbuddy', workBuddyCnAccounts.accounts),
          },
          workbuddyai: {
            ...workBuddyAiAccounts,
            accounts: orderedAccounts('workbuddyai', workBuddyAiAccounts.accounts),
          },
        },
        kiroAccounts: {
          ...kiroAccounts,
          accounts: orderedAccounts('kiro', kiroAccounts.accounts),
        },
        traeAccounts: {
          ...traeAccounts,
          accounts: orderedAccounts(
            selectedTool === 'trae' ? 'trae' : 'traecn',
            traeAccounts.accounts
          ),
        },
        traeAccountGroups: {
          trae: {
            ...traeGlobalAccounts,
            accounts: orderedAccounts('trae', traeGlobalAccounts.accounts),
          },
          traecn: {
            ...traeCnAccounts,
            accounts: orderedAccounts('traecn', traeCnAccounts.accounts),
          },
        },
        zcodeAccounts: {
          ...zcodeAccounts,
          accounts: orderedAccounts('zcode', zcodeAccounts.accounts),
        },
        kimiAccounts: {
          ...kimiAccounts,
          accounts: orderedAccounts('kimi', kimiAccounts.accounts),
        },
        codeArtsAccounts: {
          ...codeArtsAccounts,
          accounts: orderedAccounts('codearts', codeArtsAccounts.accounts),
        },
        openDesignAccounts: {
          ...openDesignAccounts,
          accounts: orderedAccounts('opendesign', openDesignAccounts.accounts),
        },
        minimaxAccounts: {
          ...minimaxAccounts,
          accounts: orderedAccounts('minimax', minimaxAccounts.accounts),
        },
        qoderAccounts: {
          ...qoderAccounts,
          accounts: orderedAccounts(qoderEdition, qoderAccounts.accounts),
        },
        qoderAccountGroups: {
          qoder: {
            ...qoderGlobalAccounts,
            accounts: orderedAccounts('qoder', qoderGlobalAccounts.accounts),
          },
          qodercn: {
            ...qoderCnAccounts,
            accounts: orderedAccounts('qodercn', qoderCnAccounts.accounts),
          },
        },
        deepSeekAccounts: {
          ...deepSeekAccounts,
          accounts: orderedAccounts('dsh', deepSeekAccounts.accounts),
        },
        grokAccounts: { ...grokAccounts, accounts: orderedAccounts('grok', grokAccounts.accounts) },
        clineAccounts: {
          ...clineAccounts,
          accounts: orderedAccounts('cline', clineAccounts.accounts),
        },
        kiloAccounts: { ...kiloAccounts, accounts: orderedAccounts('kilo', kiloAccounts.accounts) },
        copilotAccounts: {
          ...copilotAccounts,
          accounts: orderedAccounts('copilot', copilotAccounts.accounts),
        },
        manusAccounts: {
          ...manusAccounts,
          accounts: orderedAccounts('manus', manusAccounts.accounts),
        },
        cueAccounts: {
          ...cueAccounts,
          accounts: orderedAccounts('cue', cueAccounts.accounts),
        },
        grokBotAccounts: {
          ...grokBotAccounts,
          accounts: orderedAccounts('grokbot', grokBotAccounts.accounts),
        },
        cursorAccounts: {
          ...cursorAccounts,
          accounts: orderedAccounts('cursor', cursorAccounts.accounts),
        },
        antigravityAccounts: {
          ...antigravityAccounts,
          accounts: orderedAccounts('antigravity', antigravityAccounts.accounts),
        },
        codexAccounts: orderedAccounts('codex', codexAccounts),
        selectedCodexAccountId,
        setSelectedCodexAccountId: selectCodexAccount,
        isLoadingCodexAccounts,
        codexAccountsLoaded,
        codexAccountsReadFailed,
        codexAccountsMutationRevision,
        isAddingCodexAccount,
        codexOAuthRemainingSeconds,
        refreshingCodexAccountIds,
        codexAuthorizationFailedIds,
        addCodexAccount,
        cancelCodexLogin,
        refreshCodexAccountQuota,
        deleteCodexAccount,
        selectedToolData,
        applyError,
        setApplyError,
        detectedTools,
        setDetectedTools,
        isScanning,
        scanTools,
        userModels,
        codexWebSearch,
        setCodexWebSearch,
        claudeDesktopWebSearch,
        claudeCodeWebSearch,
        setClaudeWebSearch,
        claudeDesktop1mMode,
        setClaudeDesktop1mMode,
        claude1mMode,
        setClaude1mMode,
        handleLaunch,
        onGoToMother: handleGoToMother,
        aiInstallableIds,
        viewMode,
        setViewMode,
      }}
    >
      {children}
      {claudeCodeAccounts.login && (
        <ClaudeCodeLoginDialog
          submitting={claudeCodeAccounts.submitting}
          error={claudeCodeAccounts.loginError}
          onClose={claudeCodeAccounts.cancelLogin}
          onSubmit={claudeCodeAccounts.completeLogin}
        />
      )}
    </AppManagerContext.Provider>
  );
};
