import React, { useState, useRef, useEffect, useMemo } from 'react';
import { createPortal } from 'react-dom';
import { ChevronDown, RefreshCw } from 'lucide-react';

interface ModelIdComboboxProps {
  value: string;
  onChange: (value: string) => void;
  /** Suggestion list. Empty/absent → behaves as a plain text input (no dropdown). */
  options?: string[];
  placeholder?: string;
  selectOnly?: boolean;
  optionLabels?: Record<string, string>;
  disabled?: boolean;
  ariaLabel?: string;
  onRefresh?: () => void;
  refreshLabel?: string;
  refreshing?: boolean;
  /** Optional one-click paste affordance. When provided, a plain-text "paste"
   *  control (no styling/hover) renders inside the field and calls `onPaste` on
   *  click — the caller owns the clipboard read + value write. `pasteLabel` is
   *  the localized button text (caller passes its i18n string) so this generic
   *  combobox stays free of the app's i18n hook. Mirrors the paste controls on
   *  the other Model-Nexus inputs (API key / URLs). */
  onPaste?: () => void;
  pasteLabel?: string;
}

type MenuPos = {
  left: number;
  width: number;
  below: number; // viewport y for top edge when dropping down
  above: number; // distance from viewport bottom to input top, for drop-up
  dropUp: boolean;
  maxHeight: number;
};

const MENU_MAX = 240;

/**
 * Single-box searchable model-id picker. One input the user can freely type
 * into (any id, listed or not), with a filtered suggestion dropdown that opens
 * on focus / while typing when `options` are provided.
 *
 * The dropdown is rendered through a portal with fixed positioning so it is
 * never clipped by an ancestor's overflow (e.g. the Add-Model modal body).
 */
export function ModelIdCombobox({
  value,
  onChange,
  options = [],
  placeholder,
  selectOnly = false,
  optionLabels,
  disabled = false,
  ariaLabel,
  onRefresh,
  refreshLabel,
  refreshing = false,
  onPaste,
  pasteLabel,
}: ModelIdComboboxProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [activeIdx, setActiveIdx] = useState(-1);
  // Browse vs. search: focus / chevron / arrow = browse (show ALL options so
  // the full list is visible even when the field already holds a valid id);
  // typing flips to search and filters by the current text.
  const [searching, setSearching] = useState(false);
  const [menuPos, setMenuPos] = useState<MenuPos | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // Dedupe defensively so duplicate ids in the directory data can't collide on
  // the React `key` below.
  const opts = useMemo(() => Array.from(new Set(options)), [options]);
  const hasOptions = opts.length > 0;
  const hasChevron = hasOptions || selectOnly;
  const label = (id: string) => optionLabels?.[id] ?? id;
  const query = value.trim().toLowerCase();
  const filtered =
    searching && query ? opts.filter((o) => label(o).toLowerCase().includes(query)) : opts;
  const showList = !disabled && isOpen && hasOptions && filtered.length > 0;
  const padding = onRefresh
    ? hasChevron
      ? 'pr-14'
      : onPaste
        ? 'pr-24'
        : 'pr-8'
    : hasChevron
      ? 'pr-8'
      : onPaste
        ? 'pr-16'
        : '';

  if (disabled && isOpen) {
    setIsOpen(false);
    setActiveIdx(-1);
  }

  // Position the portal menu against the input, flipping up when the space
  // below is tight. Recomputed on open and on any scroll/resize while open.
  useEffect(() => {
    if (!showList) return;
    const compute = () => {
      const el = inputRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      const spaceBelow = window.innerHeight - r.bottom;
      const spaceAbove = r.top;
      const dropUp = spaceBelow < Math.min(MENU_MAX, 160) && spaceAbove > spaceBelow;
      const maxHeight = Math.max(80, Math.min(MENU_MAX, (dropUp ? spaceAbove : spaceBelow) - 8));
      setMenuPos({
        left: r.left,
        width: r.width,
        below: r.bottom + 1,
        above: window.innerHeight - r.top + 1,
        dropUp,
        maxHeight,
      });
    };
    compute();
    window.addEventListener('scroll', compute, true);
    window.addEventListener('resize', compute);
    return () => {
      window.removeEventListener('scroll', compute, true);
      window.removeEventListener('resize', compute);
    };
  }, [showList, filtered.length]);

  // Close when clicking outside (input container OR the portal menu).
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      const t = event.target as Node;
      if (containerRef.current?.contains(t)) return;
      if (menuRef.current?.contains(t)) return;
      setIsOpen(false);
      setActiveIdx(-1);
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const pick = (id: string) => {
    if (disabled) return;
    onChange(id);
    // Commit and exit input state: close the list AND drop focus. Switching
    // again is then a single click on the field — a fresh focus that reopens
    // the list (onFocus fires), with no "already-focused, click does nothing"
    // dead end.
    setIsOpen(false);
    setSearching(false);
    setActiveIdx(-1);
    inputRef.current?.blur();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (disabled) return;
    if (!showList) {
      if (e.key === 'ArrowDown' && hasOptions) {
        setSearching(false);
        setIsOpen(true);
      }
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActiveIdx((i) => Math.min(i + 1, filtered.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActiveIdx((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      if (activeIdx >= 0 && activeIdx < filtered.length) {
        e.preventDefault();
        pick(filtered[activeIdx]);
      }
    } else if (e.key === 'Escape') {
      setIsOpen(false);
      setActiveIdx(-1);
    }
  };

  return (
    <div ref={containerRef} className="relative">
      <input
        ref={inputRef}
        type="text"
        placeholder={placeholder}
        value={label(value)}
        readOnly={selectOnly}
        style={selectOnly ? { cursor: 'default' } : undefined}
        disabled={disabled}
        aria-label={ariaLabel}
        onChange={(e) => {
          if (selectOnly || disabled) return;
          onChange(e.target.value);
          setIsOpen(true);
          setSearching(true);
          setActiveIdx(-1);
        }}
        onFocus={() => {
          if (!disabled && hasOptions) {
            setIsOpen(true);
            setSearching(false);
            setActiveIdx(-1);
          }
        }}
        // Clicking the input always (re)opens the list — even when it already
        // has focus (where onFocus won't fire again), so a closed list is
        // never stuck shut.
        onClick={() => {
          if (!disabled && hasOptions) setIsOpen(true);
        }}
        onKeyDown={handleKeyDown}
        // Reserve space for the field's actions; paste is shown only for free text.
        className={`w-full bg-cyber-input border border-cyber-border ${padding} px-2 py-1.5 text-xs text-cyber-text font-mono focus:border-cyber-border focus:outline-none rounded-button`}
      />
      {hasChevron && (
        <button
          type="button"
          tabIndex={-1}
          disabled={disabled || !hasOptions}
          onClick={() => {
            if (disabled || !hasOptions) return;
            setSearching(false);
            setActiveIdx(-1);
            setIsOpen((o) => !o);
          }}
          aria-label="Toggle model id suggestions"
          className="absolute right-1.5 top-1/2 -translate-y-1/2 text-cyber-text-muted hover:text-cyber-text"
        >
          <ChevronDown size={14} className={`transition-transform ${isOpen ? 'rotate-180' : ''}`} />
        </button>
      )}
      {onRefresh && (
        <button
          type="button"
          disabled={disabled || refreshing}
          aria-label={refreshLabel}
          onMouseDown={(e) => e.preventDefault()}
          onClick={(e) => {
            e.stopPropagation();
            if (!disabled && !refreshing) onRefresh();
          }}
          className={`absolute ${hasChevron ? 'right-7' : 'right-1.5'} top-1/2 -translate-y-1/2 text-cyber-text-muted hover:text-cyber-text disabled:opacity-50`}
        >
          <RefreshCw size={14} className={refreshing ? 'animate-spin' : undefined} />
        </button>
      )}
      {onPaste && pasteLabel && !hasChevron && (
        <button
          type="button"
          tabIndex={-1}
          disabled={disabled}
          onClick={() => {
            if (!disabled) onPaste();
          }}
          className={`absolute ${onRefresh ? 'right-8' : 'right-2'} top-1/2 -translate-y-1/2 text-xs text-cyber-text-secondary`}
        >
          {pasteLabel}
        </button>
      )}
      {showList &&
        menuPos &&
        createPortal(
          <div
            ref={menuRef}
            style={{
              position: 'fixed',
              left: menuPos.left,
              width: menuPos.width,
              top: menuPos.dropUp ? undefined : menuPos.below,
              bottom: menuPos.dropUp ? menuPos.above : undefined,
              maxHeight: menuPos.maxHeight,
            }}
            className="overflow-y-auto z-[9999] bg-cyber-surface border border-cyber-border/60 rounded-button shadow-lg cursor-default"
          >
            {filtered.map((opt, idx) => (
              <div
                key={opt}
                // mousedown (not click) so selection fires before the input blurs.
                onMouseDown={(e) => {
                  e.preventDefault();
                  pick(opt);
                }}
                onMouseEnter={() => setActiveIdx(idx)}
                className={`px-2 py-1.5 text-xs font-mono truncate transition-colors ${
                  opt === value
                    ? 'bg-cyber-text/15 text-cyber-text'
                    : idx === activeIdx
                      ? 'bg-cyber-text/10 text-cyber-text'
                      : 'text-cyber-text hover:bg-cyber-text/10'
                }`}
              >
                {label(opt)}
              </div>
            ))}
          </div>,
          document.body
        )}
    </div>
  );
}
