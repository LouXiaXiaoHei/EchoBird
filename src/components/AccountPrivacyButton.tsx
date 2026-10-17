import { Eye, EyeOff } from 'lucide-react';
import { useI18n } from '../hooks/useI18n';
import { useAccountPrivacyStore } from '../stores/accountPrivacyStore';
import { PageActionButton } from './PageActionButton';

export function AccountPrivacyButton() {
  const { t } = useI18n();
  const { hidden, setHidden } = useAccountPrivacyStore();
  return (
    <PageActionButton
      icon={hidden ? EyeOff : Eye}
      onClick={() => setHidden(!hidden)}
      aria-label={t(hidden ? 'freeModels.showAccountNames' : 'freeModels.hideAccountNames')}
      aria-pressed={hidden}
    />
  );
}
