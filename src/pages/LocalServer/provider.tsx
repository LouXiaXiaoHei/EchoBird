// Local Server Page — llama-server management UI
// Layout restored from Electron v1.0.8 LocalModelPlayer.tsx
// Architecture: Provider + Main + Panel (consistent with other pages)
import { useState, useEffect, useRef, useCallback } from 'react';
import * as api from '../../api/tauri';
import { useNavigationStore } from '../../stores/navigationStore';
import { useDocumentVisible } from '../../hooks/useDocumentVisible';
import { useLocalServerStatusStore } from '../../stores/localServerStatusStore';
import { LocalServerContext } from './context';
import type { GgufFileEntry } from './context';

// ─── Provider ───

export const LocalServerProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [selectedModelPath, setSelectedModelPath] = useState<string | null>(null);
  const [ggufFiles, setGgufFiles] = useState<GgufFileEntry[]>([]);
  const [isScanning, setIsScanning] = useState(false);
  const [modelsDirs, setModelsDirs] = useState<string[]>([]);
  // Shared runtime state — Panel reads this to filter store models
  const [runtime, setRuntime] = useState('llama-server');
  // Server runtime state (shared with bottom bar)
  const serverRunning = useLocalServerStatusStore((state) => state.running);
  const setServerRunning = useLocalServerStatusStore((state) => state.setRunning);
  const docVisible = useDocumentVisible();
  const localPage = useNavigationStore((state) => state.activePage === 'localLlm');
  useEffect(() => {
    if (!docVisible) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const revision = useLocalServerStatusStore.getState().revision;
        const info = await api.getLlmServerInfo();
        if (cancelled || revision !== useLocalServerStatusStore.getState().revision) return;
        const previous = useLocalServerStatusStore.getState().running;
        setServerRunning(info.running);
        if (previous !== info.running) window.dispatchEvent(new Event('models-changed'));
      } catch {
        /* Keep the last known state on a transient IPC failure. */
      } finally {
        if (!cancelled) timer = setTimeout(poll, localPage ? 1000 : 2000);
      }
    };
    void poll();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [docVisible, localPage, setServerRunning]);
  const [serverPort, setServerPort] = useState(11434);
  const [serverModelName, setServerModelName] = useState('');
  const [serverApiKey, setServerApiKey] = useState('');

  const scanRuntime = useRef(runtime);
  const scanRequest = useRef(0);
  const rescanModels = useCallback(async (rt = scanRuntime.current) => {
    const request = ++scanRequest.current;
    setIsScanning(true);
    try {
      const dirs = await api.getModelsDirs();
      const allFiles: GgufFileEntry[] = [];
      const isHfRuntime = rt === 'vllm' || rt === 'sglang';
      for (const dir of dirs) {
        if (isHfRuntime) {
          // Scan HuggingFace model directories
          const models = await api.scanHfModels(dir);
          for (const m of models) {
            allFiles.push({
              fileName: m.modelName,
              filePath: m.modelPath,
              fileSize: m.totalSize,
            });
          }
        } else {
          // Scan GGUF files (default)
          const files = await api.scanGgufFiles(dir);
          for (const f of files) {
            allFiles.push({
              fileName: f.fileName,
              filePath: f.filePath,
              fileSize: f.fileSize,
            });
          }
        }
      }
      if (request === scanRequest.current) {
        setModelsDirs(dirs);
        setGgufFiles(Array.from(new Map(allFiles.map((file) => [file.filePath, file])).values()));
      }
    } catch (e) {
      console.error('[LocalServer] Failed to scan models:', e);
    }
    if (request === scanRequest.current) setIsScanning(false);
  }, []);

  const [activated, setActivated] = useState(localPage);
  if (localPage && !activated) setActivated(true);
  // Scan the current format and ignore earlier requests after switching engines.
  const previousRuntime = useRef(runtime);
  useEffect(() => {
    if (!activated) return;
    scanRuntime.current = runtime;
    if (previousRuntime.current !== runtime) {
      setSelectedModelPath(null);
      setGgufFiles([]);
      previousRuntime.current = runtime;
    }
    // eslint-disable-next-line react-hooks/set-state-in-effect
    rescanModels();
    const requests = scanRequest;
    return () => {
      requests.current++;
    };
  }, [runtime, rescanModels, activated]);

  return (
    <LocalServerContext.Provider
      value={{
        selectedModelPath,
        setSelectedModelPath,
        ggufFiles,
        isScanning,
        rescanModels,
        modelsDirs,
        runtime,
        setRuntime,
        serverRunning,
        setServerRunning,
        serverPort,
        setServerPort,
        serverModelName,
        setServerModelName,
        serverApiKey,
        setServerApiKey,
      }}
    >
      {children}
    </LocalServerContext.Provider>
  );
};
