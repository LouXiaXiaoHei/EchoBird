import { useCallback, useEffect, useRef, useState } from 'react';
import { Pencil, Plus, RefreshCw, Trash2 } from 'lucide-react';
import * as api from '../../api/tauri';
import { useConfirm } from '../../components/ConfirmDialog';
import { useToast } from '../../components/Toast';
import { useDocumentVisible } from '../../hooks/useDocumentVisible';
import { useI18n } from '../../hooks/useI18n';
import { useNavigationStore } from '../../stores/navigationStore';
import { useMotherAgent } from './context';

const MAX_TEXT_BYTES = 2000;

export function MemoryPanel({ active }: { active: boolean }) {
  const { selectedServerId, activeConversationId, parasiteAgent } = useMotherAgent();
  const { t } = useI18n();
  const confirm = useConfirm();
  const { showToast } = useToast();
  const isMotherActive = useNavigationStore((state) => state.activePage === 'mother');
  const documentVisible = useDocumentVisible();
  const target = selectedServerId || 'local';
  const enabled = active && isMotherActive && documentVisible && parasiteAgent !== 'claudecode';
  const [records, setRecords] = useState<api.AgentMemoryRecord[]>([]);
  const [loadError, setLoadError] = useState('');
  const [loading, setLoading] = useState(false);
  const [pending, setPending] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [text, setText] = useState('');
  const [scope, setScope] = useState<api.AgentMemoryRecord['scope']>('user');
  const view = useRef(0);
  const mounted = useRef(false);
  const request = useRef(0);
  const actionPending = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const refresh = useCallback(async (currentView: number) => {
    const currentRequest = ++request.current;
    setLoading(true);
    try {
      const saved = await api.agentMemoryList();
      if (!mounted.current || currentView !== view.current || currentRequest !== request.current)
        return;
      setRecords(saved);
      setLoadError('');
    } catch (error) {
      if (!mounted.current || currentView !== view.current || currentRequest !== request.current)
        return;
      setLoadError(String(error));
    } finally {
      if (mounted.current && currentView === view.current && currentRequest === request.current)
        setLoading(false);
    }
  }, []);

  useEffect(() => {
    const currentView = ++view.current;
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setLoadError('');
    if (enabled) void refresh(currentView);
    return () => {
      view.current += 1;
    };
  }, [enabled, target, refresh]);

  useEffect(() => {
    // Navigation and window visibility preserve drafts; only the destination resets them.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setEditing(null);
    setText('');
    setScope('user');
  }, [target, parasiteAgent]);

  const runAction = async (action: () => Promise<unknown>, after?: () => void) => {
    if (!enabled || actionPending.current) return;
    actionPending.current = true;
    setPending(true);
    const currentView = view.current;
    try {
      const result = await action();
      if (result === false) return;
      if (mounted.current && currentView === view.current) {
        after?.();
        await refresh(currentView);
      }
    } catch (error) {
      if (mounted.current && currentView === view.current) showToast('error', String(error));
    } finally {
      actionPending.current = false;
      if (mounted.current) setPending(false);
    }
  };

  const save = () => {
    const value = text.trim();
    if (!editing || !value || new TextEncoder().encode(value).length > MAX_TEXT_BYTES) return;
    void runAction(
      () =>
        editing === 'new'
          ? api.agentMemorySave(
              scope,
              scope === 'target' ? target : null,
              value,
              activeConversationId || null
            )
          : api.agentMemoryUpdate(editing, value),
      () => {
        setEditing(null);
        setText('');
      }
    );
  };

  const remove = (record: api.AgentMemoryRecord) => {
    const currentView = view.current;
    void runAction(async () => {
      const accepted = await confirm({
        title: t('mother.memory.deleteTitle'),
        message: t('mother.memory.deleteConfirm'),
        confirmText: t('btn.delete'),
        cancelText: t('btn.cancel'),
        type: 'danger',
      });
      if (!accepted || currentView !== view.current || !mounted.current) return false;
      await api.agentMemoryDelete(record.id);
      if (editing === record.id) {
        setEditing(null);
        setText('');
      }
    });
  };

  if (!active) return null;
  if (parasiteAgent === 'claudecode')
    return (
      <p className="px-3 py-2 text-xs text-cyber-text-secondary">{t('mother.memory.native')}</p>
    );

  const visibleRecords = records.filter(
    (record) => record.scope === 'user' || (record.scope === 'target' && record.target === target)
  );
  const textTooLong = new TextEncoder().encode(text.trim()).length > MAX_TEXT_BYTES;
  const inputClass =
    'w-full bg-cyber-input border border-cyber-border px-2 py-1.5 text-sm text-cyber-text focus:outline-none focus-visible:border-cyber-accent rounded-button disabled:opacity-40';

  const editor = (
    <div className="space-y-2">
      {editing === 'new' && (
        <select
          aria-label={t('mother.memory.scope')}
          value={scope}
          disabled={pending || !enabled}
          onChange={(event) =>
            setScope(event.currentTarget.value as api.AgentMemoryRecord['scope'])
          }
          className={inputClass}
        >
          <option value="user">{t('mother.memory.user')}</option>
          <option value="target">{t('mother.memory.target')}</option>
        </select>
      )}
      <textarea
        aria-label={t('mother.memory.text')}
        placeholder={t('mother.memory.placeholder')}
        value={text}
        disabled={pending || !enabled}
        rows={3}
        onChange={(event) => setText(event.currentTarget.value)}
        className={`${inputClass} resize-none`}
      />
      {textTooLong && (
        <p role="alert" className="text-xs text-cyber-error">
          {t('mother.memory.tooLong')}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <button
          type="button"
          onClick={() => {
            setEditing(null);
            setText('');
          }}
          disabled={pending}
          className="px-2 py-1.5 text-xs text-cyber-text-secondary hover:text-cyber-text transition-colors disabled:opacity-40"
        >
          {t('btn.cancel')}
        </button>
        <button
          type="button"
          onClick={save}
          disabled={pending || !enabled || !text.trim() || textTooLong}
          className="px-3 py-1.5 rounded-button text-xs font-semibold text-cyber-text bg-cyber-elevated hover:bg-cyber-text/10 transition-colors disabled:opacity-40"
        >
          {t('btn.save')}
        </button>
      </div>
    </div>
  );

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <button
          type="button"
          disabled={pending || !enabled}
          onClick={() => {
            setScope('user');
            setText('');
            setEditing('new');
          }}
          className="flex-1 h-9 px-3 text-[14px] font-semibold border border-cyber-border rounded-button transition-colors flex items-center justify-center gap-2 text-cyber-text-secondary hover:text-cyber-text hover:bg-cyber-elevated disabled:opacity-40"
        >
          <Plus size={14} />
          {t('mother.memory.add')}
        </button>
        <button
          type="button"
          aria-label={t('btn.refresh')}
          disabled={pending || loading || !enabled}
          onClick={() => void refresh(view.current)}
          className="p-2 text-cyber-text-secondary hover:text-cyber-text transition-colors disabled:opacity-40"
        >
          <RefreshCw size={14} className={loading ? 'animate-spin' : undefined} />
        </button>
      </div>
      <p className="px-1 text-xs text-cyber-text-secondary">{t('mother.memory.nextMessage')}</p>
      {loadError && (
        <p role="alert" className="px-1 text-xs text-cyber-error break-words">
          {t('mother.memory.loadFailed')} {loadError}
        </p>
      )}
      {editing === 'new' && <div className="p-3 rounded-card bg-cyber-surface">{editor}</div>}
      {visibleRecords.map((record) => (
        <div
          key={record.id}
          className="p-3 rounded-card bg-cyber-surface hover:bg-cyber-elevated transition-colors"
        >
          <div className="flex items-center justify-between gap-2 mb-2">
            <span className="text-xs text-cyber-text-secondary">
              {record.scope === 'user' ? t('mother.memory.user') : t('mother.memory.target')}
            </span>
            <div className="flex items-center gap-2">
              <button
                type="button"
                aria-label={`${t('btn.edit')} ${record.text}`}
                disabled={pending || !enabled}
                onClick={() => {
                  setEditing(record.id);
                  setText(record.text);
                }}
                className="p-0.5 text-cyber-text-muted/70 hover:text-cyber-text transition-colors disabled:opacity-40"
              >
                <Pencil size={14} />
              </button>
              <button
                type="button"
                aria-label={`${t('btn.delete')} ${record.text}`}
                disabled={pending || !enabled}
                onClick={() => remove(record)}
                className="p-0.5 text-cyber-text-muted/70 hover:text-red-500 transition-colors disabled:opacity-40"
              >
                <Trash2 size={14} />
              </button>
            </div>
          </div>
          {editing === record.id ? (
            editor
          ) : (
            <p className="text-sm text-cyber-text whitespace-pre-wrap break-words">{record.text}</p>
          )}
        </div>
      ))}
      {!loading && !loadError && visibleRecords.length === 0 && (
        <p className="px-3 py-2 text-xs text-cyber-text-secondary">{t('mother.memory.empty')}</p>
      )}
    </div>
  );
}
