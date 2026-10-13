import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from '../../api/tauri';
import { loadLocale, translate, type TKey } from '../../i18n';
import { useNavigationStore } from '../../stores/navigationStore';
import { MemoryPanel } from './MemoryPanel';
import { MotherAgentPanel } from './MotherAgentPanel';
import type { MotherAgentCtx } from './types';

let state: Partial<MotherAgentCtx>;
let locale = 'en';
let visible = true;
let renderer: ReactTestRenderer | undefined;
const confirm = vi.fn();
const showToast = vi.fn();

vi.mock('./context', () => ({ useMotherAgent: () => state }));
vi.mock('./ConversationList', () => ({ ConversationList: () => null }));
vi.mock('../../hooks/useI18n', () => ({
  useI18n: () => ({ locale, t: (key: TKey) => translate(key, locale) }),
}));
vi.mock('../../hooks/useDocumentVisible', () => ({ useDocumentVisible: () => visible }));
vi.mock('../../components/ConfirmDialog', () => ({ useConfirm: () => confirm }));
vi.mock('../../components/Toast', () => ({ useToast: () => ({ showToast }) }));
vi.mock('../../api/tauri', () => ({
  agentMemoryList: vi.fn(),
  agentMemorySave: vi.fn(),
  agentMemoryUpdate: vi.fn(),
  agentMemoryDelete: vi.fn(),
  sendAgentMessage: vi.fn(),
  abortAgent: vi.fn(),
  resetAgent: vi.fn(),
  parasiteSendMessage: vi.fn(),
  parasiteAbort: vi.fn(),
  parasiteReset: vi.fn(),
  saveSSHServer: vi.fn(),
  applyModelToTool: vi.fn(),
}));

const record = (id: string, text: string, target: string | null): api.AgentMemoryRecord => ({
  id,
  text,
  scope: target === null ? 'user' : 'target',
  target,
  source_conversation_id: 'previous-conversation',
  created_at: 100,
  updated_at: 200,
});
const saved = [
  record('preference', 'Use Chinese', null),
  record('local-fact', 'Local fact', 'local'),
  record('remote-fact', 'Remote fact', 'remote'),
  record('other-fact', 'Private other server fact', 'other'),
];

const t = (key: TKey) => translate(key, locale);
const rendered = () => JSON.stringify(renderer!.toJSON());
const button = (key: TKey) =>
  renderer!.root.findAllByType('button').find((item) => item.children.includes(t(key)))!;
const action = (label: string) => renderer!.root.findByProps({ 'aria-label': label });

async function mount(active = true) {
  await act(async () => {
    renderer = create(<MemoryPanel active={active} />);
  });
}

async function update(active = true) {
  await act(async () => renderer!.update(<MemoryPanel active={active} />));
}

async function click(key: TKey) {
  await act(async () => button(key).props.onClick());
}

function changeText(value: string) {
  act(() => renderer!.root.findByType('textarea').props.onChange({ currentTarget: { value } }));
}

async function navigateAway(kind: 'window' | 'tab' | 'page', away: boolean) {
  if (kind === 'window') {
    visible = !away;
    await update();
  } else if (kind === 'tab') {
    await update(!away);
  } else {
    await act(async () => useNavigationStore.setState({ activePage: away ? 'apps' : 'mother' }));
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function expectNoAgentActions() {
  expect(api.sendAgentMessage).not.toHaveBeenCalled();
  expect(api.abortAgent).not.toHaveBeenCalled();
  expect(api.resetAgent).not.toHaveBeenCalled();
  expect(api.parasiteSendMessage).not.toHaveBeenCalled();
  expect(api.parasiteAbort).not.toHaveBeenCalled();
  expect(api.parasiteReset).not.toHaveBeenCalled();
  expect(api.saveSSHServer).not.toHaveBeenCalled();
  expect(api.applyModelToTool).not.toHaveBeenCalled();
  expect(state.selectServer).not.toHaveBeenCalled();
  expect(state.setAgentModel).not.toHaveBeenCalled();
  expect(state.selectConversation).not.toHaveBeenCalled();
}

beforeEach(() => {
  vi.resetAllMocks();
  locale = 'en';
  visible = true;
  state = {
    selectedServerId: 'local',
    activeConversationId: 'current-conversation',
    parasiteAgent: null,
    isProcessing: false,
    sshServers: [],
    selectServer: vi.fn(),
    setAgentModel: vi.fn(),
    selectConversation: vi.fn(),
  };
  useNavigationStore.setState({ activePage: 'mother' });
  vi.mocked(api.agentMemoryList).mockResolvedValue(saved);
  vi.mocked(api.agentMemorySave).mockResolvedValue(record('new', 'New memory', null));
  vi.mocked(api.agentMemoryUpdate).mockResolvedValue(record('preference', 'Edited memory', null));
  vi.mocked(api.agentMemoryDelete).mockResolvedValue(undefined);
  confirm.mockResolvedValue(true);
});

afterEach(() => {
  if (renderer) act(() => renderer!.unmount());
  renderer = undefined;
});

describe('Mother memory navigation', () => {
  it('loads only the visible native memory tab and never writes during passive navigation', async () => {
    await mount(false);
    expect(api.agentMemoryList).not.toHaveBeenCalled();
    await update();
    expect(api.agentMemoryList).toHaveBeenCalledOnce();
    expect(rendered()).toContain('Use Chinese');
    expect(rendered()).toContain('Local fact');
    expect(rendered()).not.toContain('Remote fact');
    expect(rendered()).not.toContain('Private other server fact');
    state = { ...state, selectedServerId: 'remote' };
    await update();
    expect(api.agentMemoryList).toHaveBeenCalledTimes(2);
    expect(rendered()).toContain('Use Chinese');
    expect(rendered()).toContain('Remote fact');
    expect(rendered()).not.toContain('Local fact');
    await update(false);
    useNavigationStore.setState({ activePage: 'apps' });
    await update();
    expect(api.agentMemoryList).toHaveBeenCalledTimes(2);
    await act(async () => useNavigationStore.setState({ activePage: 'mother' }));
    expect(api.agentMemoryList).toHaveBeenCalledTimes(3);
    visible = false;
    await update();
    expect(api.agentMemoryList).toHaveBeenCalledTimes(3);
    state = { ...state, parasiteAgent: 'claudecode' };
    visible = true;
    await update();
    expect(api.agentMemoryList).toHaveBeenCalledTimes(3);
    expect(rendered()).not.toContain('Remote fact');
    expect(rendered()).toContain(t('mother.memory.native'));
    expect(api.agentMemorySave).not.toHaveBeenCalled();
    expect(api.agentMemoryUpdate).not.toHaveBeenCalled();
    expect(api.agentMemoryDelete).not.toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();
    expect(showToast).not.toHaveBeenCalled();
    expectNoAgentActions();
  });

  it('preserves cached rows and reports load failures inline without a blocking dialog', async () => {
    await mount();
    await update(false);
    vi.mocked(api.agentMemoryList).mockRejectedValueOnce('fixture read failure');
    await update();
    expect(rendered()).toContain('Local fact');
    expect(rendered()).toContain('fixture read failure');
    expect(renderer!.root.findByProps({ role: 'alert' }).children).toContain(
      'fixture read failure'
    );
    expect(confirm).not.toHaveBeenCalled();
    expect(showToast).not.toHaveBeenCalled();
    expectNoAgentActions();
  });

  it('ignores an older target response after a new target has loaded', async () => {
    const old = deferred<api.AgentMemoryRecord[]>();
    vi.mocked(api.agentMemoryList).mockReturnValueOnce(old.promise);
    await mount();
    state = { ...state, selectedServerId: 'remote' };
    await update();
    expect(rendered()).toContain('Remote fact');
    await act(async () => old.resolve([record('stale', 'Stale preference', null)]));
    expect(rendered()).toContain('Remote fact');
    expect(rendered()).not.toContain('Stale preference');
    expect(confirm).not.toHaveBeenCalled();
    expectNoAgentActions();
  });

  it('keeps an unsaved draft when hiding the window or switching tabs/pages and clears it on target change', async () => {
    await mount();
    await click('mother.memory.add');
    act(() =>
      renderer!.root.findByType('select').props.onChange({ currentTarget: { value: 'target' } })
    );
    changeText('A fact copied from another app');
    visible = false;
    await update();
    expect(renderer!.root.findByType('textarea').props.value).toBe(
      'A fact copied from another app'
    );
    expect(renderer!.root.findByType('textarea').props.disabled).toBe(true);
    visible = true;
    await update();
    expect(renderer!.root.findByType('textarea').props.value).toBe(
      'A fact copied from another app'
    );
    expect(renderer!.root.findByType('select').props.value).toBe('target');
    await update(false);
    expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
    await update();
    expect(renderer!.root.findByType('textarea').props.value).toBe(
      'A fact copied from another app'
    );
    await act(async () => useNavigationStore.setState({ activePage: 'apps' }));
    await act(async () => useNavigationStore.setState({ activePage: 'mother' }));
    expect(renderer!.root.findByType('textarea').props.value).toBe(
      'A fact copied from another app'
    );
    expect(renderer!.root.findByType('select').props.value).toBe('target');
    state = { ...state, selectedServerId: 'remote' };
    await update();
    expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
    await click('mother.memory.add');
    expect(renderer!.root.findByType('textarea').props.value).toBe('');
    expect(renderer!.root.findByType('select').props.value).toBe('user');
    changeText('Remote-only unsaved draft');
    state = { ...state, parasiteAgent: 'claudecode' };
    await update();
    state = { ...state, parasiteAgent: null };
    await update();
    expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
    expect(api.agentMemorySave).not.toHaveBeenCalled();
    expect(api.agentMemoryUpdate).not.toHaveBeenCalled();
    expect(api.agentMemoryDelete).not.toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();
    expect(showToast).not.toHaveBeenCalled();
    expectNoAgentActions();
  });

  it('makes manual refresh a read-only action and keeps the established card and cursor styles', async () => {
    await mount();
    vi.mocked(api.agentMemoryList).mockResolvedValueOnce([
      ...saved,
      record('learned', 'Newly learned preference', null),
    ]);
    await act(async () => action(t('btn.refresh')).props.onClick());
    expect(api.agentMemoryList).toHaveBeenCalledTimes(2);
    expect(rendered()).toContain('Newly learned preference');
    expect(rendered()).toContain('bg-cyber-surface hover:bg-cyber-elevated');
    expect(rendered()).not.toContain('cursor-');
    expect(rendered()).not.toContain('title');
    expect(api.agentMemorySave).not.toHaveBeenCalled();
    expectNoAgentActions();
  });

  it('adds the fourth tab without loading memory on the default conversation tab', async () => {
    await act(async () => {
      renderer = create(<MotherAgentPanel />);
    });
    expect(button('mother.memory.tab')).toBeDefined();
    expect(api.agentMemoryList).not.toHaveBeenCalled();
    await click('mother.memory.tab');
    expect(api.agentMemoryList).toHaveBeenCalledOnce();
    await click('mother.conversations');
    vi.mocked(api.agentMemoryList).mockRejectedValueOnce('reload failed');
    await click('mother.memory.tab');
    expect(rendered()).toContain('Local fact');
    expect(rendered()).toContain('reload failed');
  });

  it.each(['en', 'zh-Hans', 'zh-Hant', 'ja'])(
    'keeps the four compact tabs at the existing font size with unchanged labels in %s',
    async (language) => {
      locale = language;
      await loadLocale(locale);
      await act(async () => {
        renderer = create(<MotherAgentPanel />);
      });
      const tabs = [
        'mother.conversations',
        'mother.memory.tab',
        'mother.servers',
        'mother.sshGuide',
      ] as const;
      for (const key of tabs) {
        const tab = button(key);
        expect(tab.children).toContain(t(key));
        expect(tab.props.className).toContain('text-[14px]');
        expect(tab.props.className).toContain('flex-1 min-w-0');
        expect(tab.props.className).not.toContain('cursor-');
        expect(tab.props.title).toBeUndefined();
      }
      expect(api.agentMemoryList).not.toHaveBeenCalled();
    }
  );
});

describe('Mother memory explicit actions', () => {
  it.each(['window', 'tab', 'page'] as const)(
    'settles pending saves, edits, and deletes after leaving and returning through the %s',
    async (navigation) => {
      await mount();
      await click('mother.memory.add');
      changeText('Saved after returning');
      const writing = deferred<api.AgentMemoryRecord>();
      vi.mocked(api.agentMemorySave).mockReturnValueOnce(writing.promise);
      act(() => button('btn.save').props.onClick());
      await navigateAway(navigation, true);
      await navigateAway(navigation, false);
      const addition = record('new-memory', 'Saved after returning', null);
      vi.mocked(api.agentMemoryList).mockResolvedValueOnce([...saved, addition]);
      await act(async () => writing.resolve(addition));
      expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
      expect(rendered()).toContain('Saved after returning');
      expect(api.agentMemorySave).toHaveBeenCalledOnce();
      expect(api.agentMemoryList).toHaveBeenCalledTimes(3);

      await act(async () => action(`${t('btn.edit')} Use Chinese`).props.onClick());
      changeText('Edited after returning');
      const updating = deferred<api.AgentMemoryRecord>();
      vi.mocked(api.agentMemoryUpdate).mockReturnValueOnce(updating.promise);
      act(() => button('btn.save').props.onClick());
      await navigateAway(navigation, true);
      await navigateAway(navigation, false);
      const edited = record('preference', 'Edited after returning', null);
      vi.mocked(api.agentMemoryList).mockResolvedValueOnce([edited, addition]);
      await act(async () => updating.resolve(edited));
      expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
      expect(rendered()).toContain('Edited after returning');
      expect(api.agentMemoryUpdate).toHaveBeenCalledExactlyOnceWith(
        'preference',
        'Edited after returning'
      );
      expect(api.agentMemoryList).toHaveBeenCalledTimes(5);

      await act(async () => action(`${t('btn.edit')} Edited after returning`).props.onClick());
      const deleting = deferred<void>();
      vi.mocked(api.agentMemoryDelete).mockReturnValueOnce(deleting.promise);
      await act(async () => action(`${t('btn.delete')} Edited after returning`).props.onClick());
      vi.mocked(api.agentMemoryList).mockResolvedValueOnce([edited, addition]);
      await navigateAway(navigation, true);
      await navigateAway(navigation, false);
      vi.mocked(api.agentMemoryList).mockResolvedValueOnce([addition]);
      await act(async () => deleting.resolve());
      expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
      expect(rendered()).not.toContain('Edited after returning');
      expect(api.agentMemoryDelete).toHaveBeenCalledExactlyOnceWith('preference');
      expect(api.agentMemoryList).toHaveBeenCalledTimes(7);
      expect(showToast).not.toHaveBeenCalled();
      expectNoAgentActions();
    }
  );

  it.each(['window', 'tab', 'page'] as const)(
    'settles saves, edits, and deletes while hidden through the %s and reloads only on return',
    async (navigation) => {
      await mount();
      await click('mother.memory.add');
      changeText('Saved while hidden');
      const writing = deferred<api.AgentMemoryRecord>();
      vi.mocked(api.agentMemorySave).mockReturnValueOnce(writing.promise);
      act(() => button('btn.save').props.onClick());
      await navigateAway(navigation, true);
      const addition = record('new-memory', 'Saved while hidden', null);
      await act(async () => writing.resolve(addition));
      expect(api.agentMemoryList).toHaveBeenCalledOnce();
      vi.mocked(api.agentMemoryList).mockResolvedValueOnce([...saved, addition]);
      await navigateAway(navigation, false);
      expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
      expect(rendered()).toContain('Saved while hidden');
      expect(api.agentMemoryList).toHaveBeenCalledTimes(2);
      expect(api.agentMemorySave).toHaveBeenCalledOnce();

      await act(async () => action(`${t('btn.edit')} Use Chinese`).props.onClick());
      changeText('Edited while hidden');
      const updating = deferred<api.AgentMemoryRecord>();
      vi.mocked(api.agentMemoryUpdate).mockReturnValueOnce(updating.promise);
      act(() => button('btn.save').props.onClick());
      await navigateAway(navigation, true);
      const edited = record('preference', 'Edited while hidden', null);
      await act(async () => updating.resolve(edited));
      expect(api.agentMemoryList).toHaveBeenCalledTimes(2);
      vi.mocked(api.agentMemoryList).mockResolvedValueOnce([edited, addition]);
      await navigateAway(navigation, false);
      expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
      expect(rendered()).toContain('Edited while hidden');
      expect(api.agentMemoryList).toHaveBeenCalledTimes(3);
      expect(api.agentMemoryUpdate).toHaveBeenCalledOnce();

      await act(async () => action(`${t('btn.edit')} Edited while hidden`).props.onClick());
      const deleting = deferred<void>();
      vi.mocked(api.agentMemoryDelete).mockReturnValueOnce(deleting.promise);
      await act(async () => action(`${t('btn.delete')} Edited while hidden`).props.onClick());
      await navigateAway(navigation, true);
      await act(async () => deleting.resolve());
      expect(api.agentMemoryList).toHaveBeenCalledTimes(3);
      vi.mocked(api.agentMemoryList).mockResolvedValueOnce([addition]);
      await navigateAway(navigation, false);
      expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
      expect(rendered()).not.toContain('Edited while hidden');
      expect(api.agentMemoryList).toHaveBeenCalledTimes(4);
      expect(api.agentMemoryDelete).toHaveBeenCalledOnce();
      expect(showToast).not.toHaveBeenCalled();
      expectNoAgentActions();
    }
  );

  it.each(['window', 'tab', 'page'] as const)(
    'reports a pending edit failure after returning through the %s and keeps the draft',
    async (navigation) => {
      await mount();
      await act(async () => action(`${t('btn.edit')} Use Chinese`).props.onClick());
      changeText('Keep the failed draft');
      const writing = deferred<api.AgentMemoryRecord>();
      vi.mocked(api.agentMemoryUpdate).mockReturnValueOnce(writing.promise);
      act(() => button('btn.save').props.onClick());
      await navigateAway(navigation, true);
      await navigateAway(navigation, false);
      await act(async () => writing.reject('Returned edit failure'));
      expect(showToast).toHaveBeenCalledExactlyOnceWith('error', 'Returned edit failure');
      expect(renderer!.root.findByType('textarea').props.value).toBe('Keep the failed draft');
      expect(button('btn.save').props.disabled).toBe(false);
      expect(api.agentMemoryList).toHaveBeenCalledTimes(2);
      expect(api.agentMemoryUpdate).toHaveBeenCalledOnce();
      expectNoAgentActions();
    }
  );

  it.each(['window', 'tab', 'page'] as const)(
    'retains a hidden edit failure through the %s until the panel is visible again',
    async (navigation) => {
      await mount();
      await act(async () => action(`${t('btn.edit')} Use Chinese`).props.onClick());
      changeText('Keep the hidden failed draft');
      const writing = deferred<api.AgentMemoryRecord>();
      vi.mocked(api.agentMemoryUpdate).mockReturnValueOnce(writing.promise);
      act(() => button('btn.save').props.onClick());
      await navigateAway(navigation, true);
      await act(async () => writing.reject('Hidden edit failure'));
      expect(showToast).not.toHaveBeenCalled();
      expect(api.agentMemoryList).toHaveBeenCalledOnce();
      await navigateAway(navigation, false);
      expect(showToast).toHaveBeenCalledExactlyOnceWith('error', 'Hidden edit failure');
      expect(renderer!.root.findByType('textarea').props.value).toBe(
        'Keep the hidden failed draft'
      );
      expect(button('btn.save').props.disabled).toBe(false);
      await navigateAway(navigation, true);
      await navigateAway(navigation, false);
      expect(showToast).toHaveBeenCalledOnce();
      expect(api.agentMemoryUpdate).toHaveBeenCalledOnce();
      expectNoAgentActions();
    }
  );

  it.each(['target', 'engine'] as const)(
    'ignores old save settlement after changing %s away and back',
    async (destination) => {
      await mount();
      await click('mother.memory.add');
      changeText('Old pending draft');
      const writing = deferred<api.AgentMemoryRecord>();
      vi.mocked(api.agentMemorySave).mockReturnValueOnce(writing.promise);
      act(() => button('btn.save').props.onClick());
      state = {
        ...state,
        ...(destination === 'target'
          ? { selectedServerId: 'remote' }
          : { parasiteAgent: 'claudecode' }),
      };
      await update();
      state = { ...state, selectedServerId: 'local', parasiteAgent: null };
      await update();
      const reads = vi.mocked(api.agentMemoryList).mock.calls.length;
      await act(async () => writing.resolve(record('old', 'Old pending draft', null)));
      expect(api.agentMemoryList).toHaveBeenCalledTimes(reads);
      expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
      expect(button('mother.memory.add').props.disabled).toBe(false);
      expect(showToast).not.toHaveBeenCalled();
      expectNoAgentActions();
    }
  );

  it('discards a deferred failure when the destination changes before return', async () => {
    await mount();
    await click('mother.memory.add');
    changeText('Old hidden draft');
    const writing = deferred<api.AgentMemoryRecord>();
    vi.mocked(api.agentMemorySave).mockReturnValueOnce(writing.promise);
    act(() => button('btn.save').props.onClick());
    await navigateAway('window', true);
    await act(async () => writing.reject('Old hidden failure'));
    state = { ...state, selectedServerId: 'remote' };
    await update();
    state = { ...state, selectedServerId: 'local' };
    await update();
    await navigateAway('window', false);
    expect(showToast).not.toHaveBeenCalled();
    expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
    expectNoAgentActions();
  });

  it.each(['window', 'tab', 'page'] as const)(
    'keeps deletion confirmation valid after returning through the %s',
    async (navigation) => {
      await mount();
      const decision = deferred<boolean>();
      confirm.mockReturnValueOnce(decision.promise);
      act(() => action(`${t('btn.delete')} Local fact`).props.onClick());
      await navigateAway(navigation, true);
      await navigateAway(navigation, false);
      vi.mocked(api.agentMemoryList).mockResolvedValueOnce(
        saved.filter((item) => item.id !== 'local-fact')
      );
      await act(async () => decision.resolve(true));
      expect(api.agentMemoryDelete).toHaveBeenCalledExactlyOnceWith('local-fact');
      expect(rendered()).not.toContain('Local fact');
      expectNoAgentActions();
    }
  );

  it('does not start deletion when its confirmation finishes while hidden', async () => {
    await mount();
    const decision = deferred<boolean>();
    confirm.mockReturnValueOnce(decision.promise);
    act(() => action(`${t('btn.delete')} Local fact`).props.onClick());
    await navigateAway('window', true);
    await act(async () => decision.resolve(true));
    expect(api.agentMemoryDelete).not.toHaveBeenCalled();
    expect(api.agentMemoryList).toHaveBeenCalledOnce();
    await navigateAway('window', false);
    expect(rendered()).toContain('Local fact');
    expect(button('mother.memory.add').props.disabled).toBe(false);
    expectNoAgentActions();
  });

  it.each(['target', 'engine'] as const)(
    'invalidates deletion confirmation after changing %s away and back',
    async (destination) => {
      await mount();
      const decision = deferred<boolean>();
      confirm.mockReturnValueOnce(decision.promise);
      act(() => action(`${t('btn.delete')} Local fact`).props.onClick());
      state = {
        ...state,
        ...(destination === 'target'
          ? { selectedServerId: 'remote' }
          : { parasiteAgent: 'claudecode' }),
      };
      await update();
      state = { ...state, selectedServerId: 'local', parasiteAgent: null };
      await update();
      await act(async () => decision.resolve(true));
      expect(api.agentMemoryDelete).not.toHaveBeenCalled();
      expect(rendered()).toContain('Local fact');
      expectNoAgentActions();
    }
  );

  it('saves a global preference during a running turn without interrupting or resending', async () => {
    state = { ...state, isProcessing: true };
    await mount();
    await click('mother.memory.add');
    changeText('  Prefer concise Chinese replies  ');
    expect(button('btn.save').props.disabled).toBe(false);
    const addition = record('new-preference', 'Prefer concise Chinese replies', null);
    vi.mocked(api.agentMemoryList).mockResolvedValueOnce([...saved, addition]);
    await click('btn.save');
    expect(api.agentMemorySave).toHaveBeenCalledExactlyOnceWith(
      'user',
      null,
      'Prefer concise Chinese replies',
      'current-conversation'
    );
    expect(rendered()).toContain('Prefer concise Chinese replies');
    expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
    expectNoAgentActions();
  });

  it('saves machine facts only for the selected target', async () => {
    state = { ...state, selectedServerId: 'remote' };
    await mount();
    await click('mother.memory.add');
    act(() =>
      renderer!.root.findByType('select').props.onChange({ currentTarget: { value: 'target' } })
    );
    changeText('Install tools under /opt');
    await click('btn.save');
    expect(api.agentMemorySave).toHaveBeenCalledExactlyOnceWith(
      'target',
      'remote',
      'Install tools under /opt',
      'current-conversation'
    );
    expectNoAgentActions();
  });

  it('edits a selected record and rejects text over the UTF-8 byte limit without a write', async () => {
    await mount();
    await act(async () => action(`${t('btn.edit')} Use Chinese`).props.onClick());
    changeText('中'.repeat(667));
    expect(button('btn.save').props.disabled).toBe(true);
    await click('btn.save');
    expect(api.agentMemoryUpdate).not.toHaveBeenCalled();
    expect(rendered()).toContain(t('mother.memory.tooLong'));
    changeText('Use English when requested');
    vi.mocked(api.agentMemoryList).mockResolvedValueOnce([
      record('preference', 'Use English when requested', null),
    ]);
    await click('btn.save');
    expect(api.agentMemoryUpdate).toHaveBeenCalledExactlyOnceWith(
      'preference',
      'Use English when requested'
    );
    expect(api.agentMemorySave).not.toHaveBeenCalled();
    expect(rendered()).toContain('Use English when requested');
    expectNoAgentActions();
  });

  it('confirms deletion, leaves a canceled deletion untouched, and isolates it from chat actions', async () => {
    await mount();
    confirm.mockResolvedValueOnce(false);
    await act(async () => action(`${t('btn.delete')} Local fact`).props.onClick());
    expect(confirm).toHaveBeenCalledOnce();
    expect(api.agentMemoryDelete).not.toHaveBeenCalled();
    expect(api.agentMemoryList).toHaveBeenCalledOnce();
    vi.mocked(api.agentMemoryList).mockResolvedValueOnce(
      saved.filter((item) => item.id !== 'local-fact')
    );
    await act(async () => action(`${t('btn.delete')} Local fact`).props.onClick());
    expect(api.agentMemoryDelete).toHaveBeenCalledExactlyOnceWith('local-fact');
    expect(rendered()).not.toContain('Local fact');
    expect(rendered()).toContain('Use Chinese');
    expectNoAgentActions();
  });

  it('does not delete after confirmation resolves on a different target', async () => {
    await mount();
    const decision = deferred<boolean>();
    confirm.mockReturnValueOnce(decision.promise);
    act(() => action(`${t('btn.delete')} Local fact`).props.onClick());
    state = { ...state, selectedServerId: 'remote' };
    await update();
    await act(async () => decision.resolve(true));
    expect(api.agentMemoryDelete).not.toHaveBeenCalled();
    expect(rendered()).toContain('Remote fact');
    expectNoAgentActions();
  });

  it('prevents duplicate saves and ignores an old save completion after leaving its target', async () => {
    await mount();
    await click('mother.memory.add');
    changeText('A pending fact');
    const writing = deferred<api.AgentMemoryRecord>();
    vi.mocked(api.agentMemorySave).mockReturnValueOnce(writing.promise);
    act(() => button('btn.save').props.onClick());
    expect(button('btn.save').props.disabled).toBe(true);
    act(() => button('btn.save').props.onClick());
    expect(api.agentMemorySave).toHaveBeenCalledOnce();
    state = { ...state, selectedServerId: 'remote' };
    await update();
    expect(renderer!.root.findAllByType('textarea')).toHaveLength(0);
    await act(async () => writing.resolve(record('late', 'A pending fact', null)));
    expect(api.agentMemoryList).toHaveBeenCalledTimes(2);
    expect(rendered()).not.toContain('A pending fact');
    expect(button('mother.memory.add').props.disabled).toBe(false);
    expect(showToast).not.toHaveBeenCalled();
    expectNoAgentActions();
  });

  it('keeps a failed edit and reports its failure without a retry or clearing chat', async () => {
    await mount();
    await act(async () => action(`${t('btn.edit')} Use Chinese`).props.onClick());
    changeText('Keep this draft');
    vi.mocked(api.agentMemoryUpdate).mockRejectedValueOnce('fixture write failed');
    await click('btn.save');
    expect(showToast).toHaveBeenCalledExactlyOnceWith('error', 'fixture write failed');
    expect(renderer!.root.findByType('textarea').props.value).toBe('Keep this draft');
    expect(api.agentMemoryUpdate).toHaveBeenCalledOnce();
    expect(api.agentMemoryList).toHaveBeenCalledOnce();
    expect(rendered()).toContain('Local fact');
    expectNoAgentActions();
  });

  it.each(['en', 'zh-Hans', 'zh-Hant', 'ja'])('renders memory labels in %s', async (language) => {
    locale = language;
    await loadLocale(locale);
    await mount();
    expect(button('mother.memory.add').children).toContain(t('mother.memory.add'));
    expect(t('mother.memory.add')).not.toBe('mother.memory.add');
    expect(rendered()).toContain(t('mother.memory.user'));
    expect(rendered()).toContain(t('mother.memory.target'));
    expect(rendered()).toContain(t('mother.memory.nextMessage'));
  });
});
