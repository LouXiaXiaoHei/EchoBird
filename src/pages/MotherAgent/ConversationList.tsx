import { useEffect, useState } from 'react';
import { Plus } from 'lucide-react';
import { useI18n } from '../../hooks/useI18n';
import { useDocumentVisible } from '../../hooks/useDocumentVisible';
import { useNavigationStore } from '../../stores/navigationStore';
import { useMotherAgent } from './context';

export function ConversationList() {
  const { t, locale } = useI18n();
  const {
    conversations,
    activeConversationId,
    createConversation,
    selectConversation,
    isProcessing,
  } = useMotherAgent();
  const docVisible = useDocumentVisible();
  const isMotherActive = useNavigationStore((s) => s.activePage === 'mother');
  const [now, setNow] = useState(Date.now);

  useEffect(() => {
    if (!docVisible || !isMotherActive) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(timer);
  }, [docVisible, isMotherActive]);

  const relativeTime = new Intl.RelativeTimeFormat(locale, { numeric: 'always' });
  const formatUpdatedTime = (updatedAt: number) => {
    const minutes = Math.floor(Math.max(0, now - updatedAt) / 60_000);
    if (minutes < 1) return t('mother.justNow');
    if (minutes < 60) return relativeTime.format(-minutes, 'minute');
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return relativeTime.format(-hours, 'hour');
    const days = Math.floor(hours / 24);
    if (days < 30) return relativeTime.format(-days, 'day');
    const months = Math.floor(days / 30);
    if (months < 12) return relativeTime.format(-months, 'month');
    return relativeTime.format(-Math.floor(months / 12), 'year');
  };

  return (
    <div>
      <button
        type="button"
        onClick={() => !isProcessing && createConversation()}
        disabled={isProcessing}
        className="w-full h-9 px-3 text-[14px] font-semibold border border-cyber-border rounded-button transition-colors flex items-center justify-center gap-2 text-cyber-text-secondary hover:text-cyber-text hover:bg-cyber-elevated disabled:opacity-40"
      >
        <Plus size={14} />
        {t('mother.newConversation')}
      </button>
      <div className="mt-2 space-y-2">
        {conversations.map((conversation) => {
          const active = conversation.id === activeConversationId;
          const messageCount = conversation.messages.filter(
            (message) => message.type === 'user' || message.type === 'assistant'
          ).length;
          return (
            <button
              key={conversation.id}
              type="button"
              onClick={() => !isProcessing && selectConversation(conversation.id)}
              disabled={isProcessing}
              aria-current={active ? 'true' : undefined}
              className={`w-full p-3 rounded-card border bg-cyber-surface text-left transition-colors hover:bg-cyber-elevated disabled:opacity-40 ${
                active ? 'border-cyber-border' : 'border-transparent'
              }`}
            >
              <div className="text-sm font-semibold text-cyber-text truncate">
                {conversation.title || t('mother.newConversation')}
              </div>
              <div className="mt-1 text-xs text-cyber-text-secondary truncate">
                {t('mother.conversationMessageCount').replace('{count}', String(messageCount))}
                {' · '}
                {formatUpdatedTime(conversation.updatedAt)}
              </div>
            </button>
          );
        })}
      </div>
    </div>
  );
}
