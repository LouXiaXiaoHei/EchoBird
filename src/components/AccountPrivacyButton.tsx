import { Eye, EyeOff } from 'lucide-react';
import { useI18n } from '../hooks/useI18n';
import { useAccountPrivacyStore } from '../stores/accountPrivacyStore';

export function AccountPrivacyButton() {
  const { t } = useI18n();
  const { hidden, setHidden } = useAccountPrivacyStore();
  return (
    <button
      type="button"
      onClick={() => setHidden(!hidden)}
      aria-label={t(hidden ? 'freeModels.showAccountNames' : 'freeModels.hideAccountNames')}
      aria-pressed={hidden}
      className="flex items-center justify-center w-9 h-9 border border-cyber-border/50 rounded-md text-cyber-text hover:bg-cyber-text/10 transition-colors outline-none"
    >
      {hidden ? <EyeOff size={16} /> : <Eye size={16} />}
    </button>
  );
}
