// In-pane chat search (Ctrl+F). The corpus lives in Rust
// (services/chat_history.rs): this panel sends the query and renders at most
// `LIMIT` compact hits. Predicates follow Chatterino's search popup:
// from:name, in:channel, badge:name, subtier:N, has:link, regex:pattern,
// is:sub|highlighted|system|first-msg|cheer-msg|redemption|reply|shared|
// deleted|mention, each negatable with a leading "!".
//
// Visual language mirrors CommandAutocomplete: an `.sn-popover` glass panel
// with a labelled header and kbd hints, a neumorphic input, and rounded rows.

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { motion } from 'framer-motion';
import { Search, X } from 'lucide-react';
import { Logger } from '../../utils/logger';

export interface ChatSearchHit {
  id: string;
  channel: string;
  ts_ms: number;
  login: string;
  display_name: string;
  user_id: string;
  content: string;
  color?: string;
  flags: number;
}

interface ChatSearchBarProps {
  /** Slice key of the pane (bare Twitch login or provider:channel); null
   *  searches every joined channel. */
  channelKey: string | null;
  /** Jump the list to a message id. Returns false when it is no longer in
   *  the pane's buffer (the ring keeps more than the pane shows). */
  onJumpTo: (messageId: string) => boolean;
  onClose: () => void;
}

const LIMIT = 100;
const DEBOUNCE_MS = 120;
const F_DELETED = 1 << 15;

/** Clickable syntax hints, appended to the query. */
const HINTS: Array<{ label: string; insert: string; title: string }> = [
  { label: 'from:', insert: 'from:', title: 'Messages from a user (comma for several)' },
  { label: 'is:sub', insert: 'is:sub ', title: 'is: sub, highlighted, system, first-msg, cheer-msg, redemption, reply, shared, deleted, mention' },
  { label: 'badge:', insert: 'badge:', title: 'Sender wears a badge, e.g. badge:moderator' },
  { label: 'has:link', insert: 'has:link ', title: 'Messages containing a link' },
  { label: 'regex:', insert: 'regex:', title: 'Regular expression (quote it if it has spaces)' },
  { label: '!', insert: '!', title: 'Negate the next term, e.g. !from:nightbot' },
];

const Kbd = ({ children }: { children: string }) => (
  <kbd className="rounded border border-white/10 bg-white/10 px-1.5 py-0.5 font-mono text-[9px] leading-none tracking-widest text-white shadow-sm">
    {children}
  </kbd>
);

function formatTime(ts: number): string {
  if (!ts) return '';
  return new Date(ts).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}

export default function ChatSearchBar({ channelKey, onJumpTo, onClose }: ChatSearchBarProps) {
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<ChatSearchHit[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const [missing, setMissing] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const seq = useRef(0);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    const q = query.trim();
    // An empty query shows nothing (see visibleHits); no state write here.
    if (!q) return;
    const mySeq = ++seq.current;
    const t = setTimeout(async () => {
      try {
        const res = await invoke<ChatSearchHit[]>('search_chat', {
          channel: channelKey,
          query: q,
          limit: LIMIT,
        });
        if (seq.current !== mySeq) return;
        setHits(res);
        setError(null);
        setActive(0);
        setMissing(null);
      } catch (e) {
        if (seq.current !== mySeq) return;
        setHits([]);
        setError(typeof e === 'string' ? e : 'Search failed');
        Logger.warn('[ChatSearch] search_chat failed:', e);
      }
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [query, channelKey]);

  // Keep the active row in view while arrowing through results.
  useEffect(() => {
    const el = listRef.current?.children[active] as HTMLElement | undefined;
    el?.scrollIntoView({ block: 'nearest' });
  }, [active]);

  const hasQuery = query.trim().length > 0;
  const visibleHits = hasQuery ? hits : [];
  const visibleError = hasQuery ? error : null;

  const jump = useCallback(
    (hit: ChatSearchHit) => {
      const ok = onJumpTo(hit.id);
      setMissing(ok ? null : hit.id);
    },
    [onJumpTo],
  );

  const insertHint = (text: string) => {
    setQuery((q) => (q.endsWith(' ') || q.length === 0 ? q + text : `${q} ${text}`));
    inputRef.current?.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActive((i) => Math.min(visibleHits.length - 1, i + 1));
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActive((i) => Math.max(0, i - 1));
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      const hit = visibleHits[active];
      if (hit) jump(hit);
    }
  };

  const scopeLabel = channelKey ? `in #${channelKey.replace(/^[a-z]+:/, '')}` : 'in every open chat';
  const countLabel = !hasQuery
    ? ''
    : visibleHits.length >= LIMIT
      ? `${LIMIT}+ matches`
      : `${visibleHits.length} ${visibleHits.length === 1 ? 'match' : 'matches'}`;

  return (
    <motion.div
      initial={{ opacity: 0, y: -6, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={{ duration: 0.15, ease: 'easeOut' }}
      className="sn-popover absolute left-2 right-2 top-2 z-30 flex max-h-[60%] origin-top flex-col overflow-hidden"
      role="search"
    >
      {/* Header: label, scope, kbd hints, close */}
      <div className="relative z-10 flex items-center justify-between border-b border-white/5 bg-background/[0.5] px-3 py-2 backdrop-blur-md">
        <div className="flex min-w-0 items-baseline gap-2">
          <span className="text-[11px] font-semibold uppercase tracking-wider text-white/50">Search chat</span>
          <span className="truncate text-[10px] text-white/35">{scopeLabel}</span>
        </div>
        <div className="flex flex-shrink-0 items-center gap-2">
          <div className="hidden items-center gap-1 opacity-60 sm:flex">
            <Kbd>↑↓</Kbd>
            <Kbd>ENTER</Kbd>
            <span className="text-[10px] text-white">jump</span>
            <Kbd>ESC</Kbd>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="grid h-5 w-5 place-items-center rounded text-white/50 transition-colors hover:bg-white/10 hover:text-white"
            aria-label="Close search"
          >
            <X size={12} />
          </button>
        </div>
      </div>

      {/* Query row */}
      <div className="px-3 pb-2 pt-2.5">
        <label className="glass-input flex items-center gap-2 px-3 py-1.5 focus-within:border-white/20">
          <Search size={13} className="flex-shrink-0 text-textSecondary" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Type a word, a name, or a filter"
            className="min-w-0 flex-1 bg-transparent text-[13px] text-textPrimary outline-none placeholder:text-textSecondary/60"
            spellCheck={false}
            aria-label="Search chat messages"
          />
          {countLabel && (
            <span className="flex-shrink-0 text-[10px] tabular-nums text-textSecondary">{countLabel}</span>
          )}
        </label>
        {!hasQuery && (
          <div className="mt-2 flex flex-wrap items-center gap-1">
            {HINTS.map((h) => (
              <button
                key={h.label}
                type="button"
                title={h.title}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => insertHint(h.insert)}
                className="rounded-[4px] border border-white/5 bg-white/5 px-1.5 py-[2px] font-mono text-[10px] text-white/60 transition-colors hover:bg-white/10 hover:text-white"
              >
                {h.label}
              </button>
            ))}
          </div>
        )}
      </div>

      {visibleError && <div className="px-4 pb-3 text-xs text-error">{visibleError}</div>}
      {!visibleError && hasQuery && visibleHits.length === 0 && (
        <div className="px-4 pb-3 text-xs text-textSecondary">Nothing in recent history matches.</div>
      )}

      {visibleHits.length > 0 && (
        <ul ref={listRef} className="custom-scrollbar min-h-0 flex-1 overflow-y-auto border-t border-white/5 p-1" role="listbox">
          {visibleHits.map((h, i) => (
            <li
              key={`${h.channel}:${h.id}`}
              role="option"
              aria-selected={i === active}
              onMouseEnter={() => setActive(i)}
              onClick={() => jump(h)}
              className={`my-0.5 flex cursor-pointer items-baseline gap-2 rounded-lg border px-3 py-1.5 transition-all duration-150 ${
                i === active
                  ? 'border-white/10 bg-white/5 shadow-[inset_0_0_20px_rgba(255,255,255,0.02)]'
                  : 'border-transparent hover:bg-white/[0.03]'
              } ${h.flags & F_DELETED ? 'opacity-50' : ''}`}
            >
              <span className="w-[52px] flex-shrink-0 font-mono text-[10px] tabular-nums text-white/35">
                {formatTime(h.ts_ms)}
              </span>
              {channelKey === null && (
                <span className="flex-shrink-0 text-[10px] text-white/40">#{h.channel.replace(/^[a-z]+:/, '')}</span>
              )}
              <span
                className="flex-shrink-0 text-[12px] font-semibold"
                style={h.color ? { color: h.color } : undefined}
              >
                {h.display_name || h.login}
              </span>
              <span className={`min-w-0 flex-1 truncate text-[12px] text-textPrimary/90 ${h.flags & F_DELETED ? 'line-through' : ''}`}>
                {h.content}
              </span>
              {missing === h.id && (
                <span className="flex-shrink-0 rounded bg-white/5 px-1.5 py-[1px] text-[9px] uppercase tracking-wide text-white/50">
                  not in view
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
    </motion.div>
  );
}
