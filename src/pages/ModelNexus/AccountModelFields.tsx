import { useCallback, useEffect, useRef, useState } from 'react';
import {
  accountProxyProviders,
  cacheAccountProxyModels,
  getAccountProxyModels,
  getCachedAccountProxyModels,
  listProxyAccounts,
  type ProxyAccount,
} from '../../api/accountProxy';
import type { AccountProvider, AccountSource } from '../../api/types';
import { ModelIdCombobox } from '../../components/ModelIdCombobox';
import { useI18n } from '../../hooks/useI18n';
import { useToast } from '../../components/Toast';
import { accountPlanLabel } from '../AppManager/accountPlanLabel';
import { codexPlanLabel } from '../AppManager/codexPlanLabel';
import type { NewModelForm } from './context';

export function AccountModelFields({
  source,
  modelId,
  modelIdOptions,
  setForm,
  active = true,
}: {
  source: AccountSource;
  modelId: string;
  modelIdOptions?: string[];
  setForm: React.Dispatch<React.SetStateAction<NewModelForm>>;
  active?: boolean;
}) {
  const { t } = useI18n();
  const { showToast } = useToast();
  const { provider, accountId } = source;
  const [accounts, setAccounts] = useState<ProxyAccount[]>([]);
  const [loadedProvider, setLoadedProvider] = useState<AccountProvider | null>(null);
  const loading = loadedProvider !== provider;
  const [fetching, setFetching] = useState(false);
  const fetchingRef = useRef(false);
  const revision = useRef(0);

  useEffect(() => {
    if (!active) return;
    let ignore = false;
    listProxyAccounts(provider)
      .then((rows) => {
        if (!ignore) setAccounts(rows);
      })
      .catch((error) => console.error('Load proxy accounts failed:', error))
      .finally(() => {
        if (!ignore) setLoadedProvider(provider);
      });
    return () => {
      ignore = true;
    };
  }, [provider, active]);

  const applyModels = useCallback(
    (models: string[]) => {
      setForm((form) => {
        if (form.accountSource?.provider !== provider || form.accountSource.accountId !== accountId)
          return form;
        return {
          ...form,
          modelIdOptions: models,
          modelId: models.includes(form.modelId) ? form.modelId : (models[0] ?? ''),
        };
      });
    },
    [provider, accountId, setForm]
  );
  const fetchModels = useCallback(async () => {
    if (!active || !accountId || fetchingRef.current) return;
    const attempt = ++revision.current;
    fetchingRef.current = true;
    setFetching(true);
    try {
      const models = await getAccountProxyModels({ provider, accountId });
      if (attempt !== revision.current) return;
      cacheAccountProxyModels({ provider, accountId }, models);
      applyModels(models);
    } catch (error) {
      if (attempt === revision.current) showToast('error', String(error));
    } finally {
      if (attempt === revision.current) {
        fetchingRef.current = false;
        setFetching(false);
      }
    }
  }, [provider, accountId, active, applyModels, showToast]);

  useEffect(() => {
    const attempt = ++revision.current;
    fetchingRef.current = false;
    void Promise.resolve().then(() => {
      if (attempt !== revision.current) return;
      setFetching(false);
      if (active && accountId) {
        const cached = getCachedAccountProxyModels({ provider, accountId });
        if (cached !== undefined) applyModels(cached);
        else void fetchModels();
      }
    });
    return () => {
      revision.current += 1;
      fetchingRef.current = false;
    };
  }, [provider, accountId, active, applyModels, fetchModels]);

  const selectSource = (next: AccountSource) => {
    if (!active) return;
    if (next.provider === provider && next.accountId === accountId) return;
    revision.current += 1;
    fetchingRef.current = false;
    setFetching(false);
    setForm((form) => ({
      ...form,
      accountSource: next,
      modelId: '',
      modelIdOptions: [],
    }));
  };
  const savedAccounts = accounts.filter((account) => account.source.provider === provider);
  return (
    <>
      <div>
        <label className="block text-xs text-cyber-text-secondary mb-1">
          {t('accountProxy.provider')}
        </label>
        <ModelIdCombobox
          selectOnly
          ariaLabel={t('accountProxy.provider')}
          disabled={!active}
          value={provider}
          options={accountProxyProviders.map((entry) => entry.id)}
          optionLabels={Object.fromEntries(
            accountProxyProviders.map((entry) => [entry.id, entry.name])
          )}
          onChange={(value) => selectSource({ provider: value as AccountProvider, accountId: '' })}
        />
      </div>
      <div>
        <label className="block text-xs text-cyber-text-secondary mb-1">
          {t('accountProxy.account')}
        </label>
        <ModelIdCombobox
          selectOnly
          ariaLabel={t('accountProxy.account')}
          disabled={!active || loading}
          placeholder="—"
          value={
            savedAccounts.some((account) => account.source.accountId === accountId) ? accountId : ''
          }
          options={savedAccounts.map((account) => account.source.accountId)}
          optionLabels={Object.fromEntries(
            savedAccounts.map((account) => {
              const plan =
                account.source.provider === 'codex'
                  ? codexPlanLabel(account.plan)
                  : account.plan
                    ? accountPlanLabel(account.plan)
                    : '';
              return [account.source.accountId, `${account.label}${plan ? ` · ${plan}` : ''}`];
            })
          )}
          onChange={(value) => selectSource({ provider, accountId: value })}
        />
      </div>
      <div>
        <label className="block text-xs text-cyber-text-secondary mb-1">{t('model.modelId')}</label>
        <ModelIdCombobox
          selectOnly
          ariaLabel={t('model.modelId')}
          value={modelId}
          options={modelIdOptions}
          placeholder="—"
          disabled={!active || !accountId || fetching}
          onChange={(value) => setForm((form) => ({ ...form, modelId: value }))}
          onRefresh={() => void fetchModels()}
          refreshLabel={t('btn.refresh')}
          refreshing={fetching}
        />
      </div>
    </>
  );
}
