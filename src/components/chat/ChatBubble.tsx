// ChatBubble — modern AI conversation style.
// Assistant: full-width markdown, no bubble (plain text on the page bg).
// User: subtle right-aligned card. Streaming caret while the agent types.
import { memo, useEffect, useState, useMemo } from 'react';
import { Paperclip, KeyRound, Image as ImageIcon } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { getModelIcon } from '../cards/ModelCard';
import { useI18n } from '../../hooks/useI18n';
import { getSpinnerVerbs, formatSpinnerVerb } from '../../i18n/spinnerVerbs';
import { mdComponents } from '../../pages/MotherAgent/mdComponents';
import { IS_LINUX } from '../../utils/platform';

export type BubbleRole = 'user' | 'assistant' | 'system' | 'error' | 'working' | 'skeleton';

export interface BubbleChip {
  type: 'file' | 'image' | 'model';
  name: string;
  /** For model chips: model provider name to look up icon */
  modelId?: string;
  /** For image chips: base64 preview data URL */
  preview?: string;
}

export interface ChatBubbleProps {
  role: BubbleRole;
  content: string;
  /** Kept for API compatibility — only Mother Agent renders here now. */
  variant?: 'mother';
  chips?: BubbleChip[];
  isStreaming?: boolean;
  subContent?: string;
}

// ── Icon-only readonly chips below user message ───────────────────────────────
const BASE_CHIP = 'flex items-center justify-center w-6 h-6 rounded border flex-shrink-0';
const CHIP_MODEL = `${BASE_CHIP} bg-cyber-text/10 border-cyber-border/40`;
const CHIP_FILE = `${BASE_CHIP} bg-cyber-bg/60 border-cyber-text-muted/30`;

function ReadonlyChips({ chips }: { chips: BubbleChip[] }) {
  if (!chips.length) return null;
  return (
    <div className="flex flex-wrap gap-1 mt-1.5">
      {chips.map((c, i) => {
        if (c.type === 'model') {
          const icon = getModelIcon('', c.modelId || '');
          return (
            <span key={i} className={CHIP_MODEL}>
              {icon ? (
                <img src={icon} alt="" className="w-4 h-4" />
              ) : (
                <KeyRound size={12} className="text-cyber-accent" />
              )}
            </span>
          );
        }
        if (c.type === 'image') {
          return (
            <span key={i} className={CHIP_FILE}>
              {c.preview ? (
                <img src={c.preview} alt={c.name} className="w-4 h-4 object-cover rounded" />
              ) : (
                <ImageIcon size={12} className="text-cyber-text-muted" />
              )}
            </span>
          );
        }
        return (
          <span key={i} className={CHIP_FILE}>
            <Paperclip size={12} className="text-cyber-text-muted" />
          </span>
        );
      })}
    </div>
  );
}

// ── Six-dot "thinking" indicator, matching Coffee-CLI ─────────────────────────
// Per-locale verb arrays + format wrapper live in src/i18n/spinnerVerbs.ts
// — moved out so non-en/zh locales (zh-Hant, ja) get native flavor verbs
// instead of falling through to the English list mid-conversation.

// Keep Linux's static indicator to avoid WebKitGTK's animation CPU cost.
// Pick a fresh verb each time the indicator mounts (each new agent turn).
function InputDotsStatic() {
  const { locale } = useI18n();
  const verbs = getSpinnerVerbs(locale);
  const formatVerb = (v: string) => formatSpinnerVerb(v, locale);

  const staticVerb = useMemo(() => {
    // eslint-disable-next-line react-hooks/purity
    const pickRandom = () => formatVerb(verbs[Math.floor(Math.random() * verbs.length)]);
    return pickRandom();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [locale]);

  return (
    <span className="inline-flex items-center gap-2">
      <span
        className="inline-block w-3 translate-y-[2px] text-center font-mono text-base leading-none text-cyber-accent"
        aria-hidden="true"
      >
        ⠋
      </span>
      <span className="font-mono text-sm text-cyber-accent">{staticVerb}</span>
    </span>
  );
}

function InputDots() {
  if (IS_LINUX) return <InputDotsStatic />;
  return <InputDotsAnimated />;
}

function InputDotsAnimated() {
  const { locale } = useI18n();
  const verbs = getSpinnerVerbs(locale);
  const formatVerb = (v: string) => formatSpinnerVerb(v, locale);
  const pickRandom = () => formatVerb(verbs[Math.floor(Math.random() * verbs.length)]);

  // Typewriter cycle: show → erase → type a new verb → repeat
  const [target, setTarget] = useState<string>(pickRandom);
  const [shown, setShown] = useState<string>(target);
  const [phase, setPhase] = useState<'show' | 'erase' | 'type'>('show');

  // When locale flips, immediately swap to a fresh verb in the new language.

  useEffect(() => {
    const next = pickRandom();
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setTarget(next);
    setShown(next);
    setPhase('show');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [locale]);

  useEffect(() => {
    if (phase === 'show') {
      const id = setTimeout(() => setPhase('erase'), 2800);
      return () => clearTimeout(id);
    }
    if (phase === 'erase') {
      if (shown.length === 0) {
        // eslint-disable-next-line react-hooks/set-state-in-effect
        setTarget(pickRandom());
        setPhase('type');
        return;
      }
      const id = setTimeout(() => setShown((s) => s.slice(0, -1)), 45);
      return () => clearTimeout(id);
    }
    // phase === 'type'
    if (shown.length >= target.length) {
      setPhase('show');
      return;
    }
    const id = setTimeout(() => setShown(target.slice(0, shown.length + 1)), 70);
    return () => clearTimeout(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase, shown, target]);

  return (
    <span className="inline-flex items-center gap-2">
      <span
        className="inline-block w-3 translate-y-[2px] text-center font-mono text-base leading-none text-cyber-accent"
        aria-hidden="true"
      >
        <span className="spinner-glyph" />
      </span>
      <span className="inline-flex items-baseline font-mono text-sm">
        {/* Preserve the verb's height while the visible text is erased. */}
        <span className="invisible w-0 shrink-0 whitespace-nowrap" aria-hidden="true">
          {target}
        </span>
        <span className="spinner-shimmer">{shown}</span>
        {/* Caret only appears while we're actively rewriting the verb
                    (erase/type). During the steady "show" phase it's hidden so
                    the line doesn't sit there with a blinking cursor on idle. */}
        {phase !== 'show' && (
          <span
            className="inline-block w-[0.5em] h-[1em] ml-0.5 bg-cyber-accent self-center"
            style={{ animation: 'caretBlink 1s steps(2) infinite' }}
          />
        )}
      </span>
    </span>
  );
}

// ── Content size guard (markdown blow-up protection) ──────────────────────────
const CONTENT_LIMIT = 12000;
const truncate = (text: string) =>
  text.length > CONTENT_LIMIT ? text.slice(0, CONTENT_LIMIT) + '…' : text;

// Strip the agent's internal scaffolding while keeping the user-facing markdown.
// `<think>...</think>` is private reasoning. `<chat>...</chat>` is a wrapper the
// system prompt asks the agent to emit; we keep the inside but drop the tags.
function cleanAgentText(content: string): string {
  return content
    .replace(/<think>[\s\S]*?<\/think>/gi, '')
    .replace(/<think>[\s\S]*$/gi, '')
    .replace(/<\/?chat[^>]*>/gi, '')
    .trim();
}

// ── Main ChatBubble ───────────────────────────────────────────────────────────
export const ChatBubble = memo(function ChatBubble({
  role,
  content,
  chips = [],
  isStreaming = false,
  subContent,
}: ChatBubbleProps) {
  const { t: _t } = useI18n();

  if (role === 'skeleton') {
    return (
      <div className="mb-6 space-y-2">
        {[80, 60, 40].map((w, i) => (
          <div
            key={i}
            className="h-3 rounded-full animate-pulse"
            style={{ width: `${w}%`, background: 'rgba(255,255,255,0.08)' }}
          />
        ))}
      </div>
    );
  }

  if (role === 'system') {
    return (
      <div className="flex justify-center my-4">
        <span className="text-cyber-text-muted/50 text-xs font-mono">{content}</span>
      </div>
    );
  }

  if (role === 'error') {
    return (
      <div className="flex flex-col items-center gap-0.5 my-4">
        <span className="text-red-400 text-xs font-mono text-center">{content}</span>
        {subContent && (
          <span className="text-red-400/60 text-[11px] font-mono text-center">{subContent}</span>
        )}
      </div>
    );
  }

  if (role === 'working') {
    return (
      <div className="flex justify-start my-4">
        <InputDots />
      </div>
    );
  }

  // ── Assistant: full-width markdown, no bubble ─────────────────────────────
  if (role === 'assistant') {
    const cleaned = cleanAgentText(content);

    // Empty + not streaming → don't render a ghost row
    if (!cleaned && !isStreaming) return null;

    return (
      <div className="mb-6 text-sm leading-relaxed text-cyber-text font-sans">
        {cleaned ? (
          <div className="markdown-body">
            <ReactMarkdown remarkPlugins={[remarkGfm]} components={mdComponents}>
              {truncate(cleaned)}
            </ReactMarkdown>
            {isStreaming && (
              <span
                className="inline-block w-1.5 h-4 ml-0.5 align-text-bottom bg-cyber-accent"
                style={IS_LINUX ? undefined : { animation: 'caretBlink 1s steps(2) infinite' }}
              />
            )}
          </div>
        ) : (
          <InputDots />
        )}
      </div>
    );
  }

  // ── User: subtle right-aligned card, plain text ──────────────────────────
  return (
    <div className="flex flex-col items-end mb-6">
      <div
        className="max-w-[78%] rounded-lg px-3.5 py-2 text-sm leading-relaxed font-sans whitespace-pre-line text-cyber-text border border-cyber-border/25 bg-cyber-text/5"
        style={{ overflowWrap: 'anywhere', wordBreak: 'break-word' }}
      >
        {truncate(content)}
      </div>
      {chips.length > 0 && <ReadonlyChips chips={chips} />}
    </div>
  );
});
