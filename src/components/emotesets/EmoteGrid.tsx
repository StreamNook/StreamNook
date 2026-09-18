// The selected set's emotes: a grid with in-set search, pagination, and
// per-emote actions (rename alias, toggle zero-width, remove, copy link, open
// on 7tv.app). Writes are optimistic with a reload on failure. Read-only when
// the user lacks manage permission on the channel.
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  Search, Trash2, Pencil, Check, X, Layers as LayersIcon, ExternalLink,
  Copy, Loader2, ChevronLeft, ChevronRight,
} from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { Tooltip } from '../ui/Tooltip';
import {
  getSetEmotes, removeEmote, renameEmote, setEmoteZeroWidth, emoteCdnUrl,
  SevenTVSessionExpired, type SetEmote,
} from '../../services/seventvEditorService';

const PER_PAGE = 100;

interface Props {
  setId: string;
  canManage: boolean;
  reloadKey: number;
  onCountChange: (count: number) => void;
  onOpenDetail: (e: SetEmote) => void;
}

export default function EmoteGrid({ setId, canManage, reloadKey, onCountChange, onOpenDetail }: Props) {
  const addToast = useAppStore((s) => s.addToast);
  const [emotes, setEmotes] = useState<SetEmote[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [page, setPage] = useState(1);
  const [pageCount, setPageCount] = useState(1);
  const [query, setQuery] = useState('');
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [activeQuery, setActiveQuery] = useState('');
  // Guards against a slower earlier fetch (e.g. previous set/page) resolving
  // after a newer one and overwriting the grid with stale emotes.
  const reqIdRef = useRef(0);

  const load = useCallback(async () => {
    const myReq = ++reqIdRef.current;
    setLoading(true);
    setError(null);
    try {
      const res = await getSetEmotes(setId, page, PER_PAGE, activeQuery || undefined);
      if (myReq !== reqIdRef.current) return;
      setEmotes(res.emotes);
      setPageCount(Math.max(1, res.pageCount));
      onCountChange(res.totalCount);
    } catch (e) {
      if (myReq !== reqIdRef.current) return;
      if (e instanceof SevenTVSessionExpired) {
        addToast('Your 7TV session expired. Reconnect your 7TV account.', 'error');
      }
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (myReq === reqIdRef.current) setLoading(false);
    }
  }, [setId, page, activeQuery, onCountChange, addToast]);

  useEffect(() => {
    load();
  }, [load, reloadKey]);

  // Reset to page 1 when the set or query changes.
  useEffect(() => {
    setPage(1);
  }, [setId, activeQuery]);

  // Drop any pending filter debounce on unmount.
  useEffect(() => () => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
  }, []);

  const onQueryChange = (v: string) => {
    setQuery(v);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => setActiveQuery(v.trim()), 300);
  };

  const handleRemove = async (e: SetEmote) => {
    const prev = emotes;
    setEmotes((cur) => cur.filter((x) => !(x.emoteId === e.emoteId && x.alias === e.alias)));
    onCountChange(Math.max(0, prev.length - 1));
    try {
      await removeEmote(setId, e.emoteId, e.alias);
    } catch (err) {
      setEmotes(prev);
      if (err instanceof SevenTVSessionExpired) {
        addToast('Your 7TV session expired. Reconnect your 7TV account.', 'error');
      } else {
        addToast(err instanceof Error ? err.message : 'Could not remove emote', 'error');
      }
      load();
    }
  };

  const handleRename = async (e: SetEmote, newAlias: string) => {
    const trimmed = newAlias.trim();
    if (!trimmed || trimmed === e.alias) return;
    const prev = emotes;
    setEmotes((cur) => cur.map((x) => (x.emoteId === e.emoteId && x.alias === e.alias ? { ...x, alias: trimmed } : x)));
    try {
      await renameEmote(setId, e.emoteId, trimmed, e.alias);
    } catch (err) {
      setEmotes(prev);
      addToast(
        err instanceof SevenTVSessionExpired
          ? 'Your 7TV session expired. Reconnect your 7TV account.'
          : err instanceof Error ? err.message : 'Could not rename emote',
        'error',
      );
    }
  };

  const handleZeroWidth = async (e: SetEmote) => {
    const next = !e.zeroWidth;
    const prev = emotes;
    setEmotes((cur) => cur.map((x) => (x.emoteId === e.emoteId && x.alias === e.alias ? { ...x, zeroWidth: next } : x)));
    try {
      await setEmoteZeroWidth(setId, e.emoteId, next, e.alias);
    } catch (err) {
      setEmotes(prev);
      addToast(
        err instanceof SevenTVSessionExpired
          ? 'Your 7TV session expired. Reconnect your 7TV account.'
          : err instanceof Error ? err.message : 'Could not update emote',
        'error',
      );
    }
  };

  return (
    <div className="h-full flex flex-col">
      {/* In-set search */}
      <div className="px-4 py-2 shrink-0">
        <div className="relative max-w-sm">
          <Search size={14} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-textMuted" />
          <input
            value={query}
            onChange={(e) => onQueryChange(e.target.value)}
            placeholder="Filter emotes in this set"
            className="w-full pl-8 pr-3 py-1.5 rounded-lg bg-glass text-sm text-textPrimary placeholder:text-textMuted outline-none focus:ring-1 focus:ring-accent/40"
          />
        </div>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto custom-scrollbar px-4 pb-3">
        {loading ? (
          <div className="h-full flex items-center justify-center">
            <Loader2 size={20} className="animate-spin text-textSecondary" />
          </div>
        ) : error ? (
          <div className="h-full flex items-center justify-center">
            <p className="text-sm text-textSecondary max-w-md text-center">{error}</p>
          </div>
        ) : emotes.length === 0 ? (
          <div className="h-full flex items-center justify-center">
            <p className="text-sm text-textMuted">
              {activeQuery ? 'No emotes match that filter.' : 'This set is empty.'}
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(112px,1fr))] gap-2">
            {emotes.map((e) => (
              <EmoteTile
                key={`${e.emoteId}:${e.alias}`}
                emote={e}
                canManage={canManage}
                onRemove={() => handleRemove(e)}
                onRename={(alias) => handleRename(e, alias)}
                onToggleZeroWidth={() => handleZeroWidth(e)}
                onOpenDetail={() => onOpenDetail(e)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Pagination */}
      {pageCount > 1 && (
        // The same flipper as the title bar, with the page count in the slot
        // the divider usually occupies. Paging is navigation, so it gets the
        // navigation control rather than a private pair of buttons.
        <div className="flex items-center justify-center py-2 border-t border-borderSubtle shrink-0">
          <div className="chrome-glaze nav-flipper">
            <button
              type="button"
              disabled={page <= 1}
              onClick={() => setPage((p) => Math.max(1, p - 1))}
              className="nav-flipper-btn"
              aria-label="Previous page"
            >
              <ChevronLeft size={20} />
            </button>
            <span className="nav-flipper-label">
              {page} / {pageCount}
            </span>
            <button
              type="button"
              disabled={page >= pageCount}
              onClick={() => setPage((p) => Math.min(pageCount, p + 1))}
              className="nav-flipper-btn"
              aria-label="Next page"
            >
              <ChevronRight size={20} />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function EmoteTile({
  emote,
  canManage,
  onRemove,
  onRename,
  onToggleZeroWidth,
  onOpenDetail,
}: {
  emote: SetEmote;
  canManage: boolean;
  onRemove: () => void;
  onRename: (alias: string) => void;
  onToggleZeroWidth: () => void;
  onOpenDetail: () => void;
}) {
  const addToast = useAppStore((s) => s.addToast);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(emote.alias);
  const [fallback, setFallback] = useState(false);

  const src = fallback
    ? `https://cdn.7tv.app/emote/${emote.emoteId}/3x.webp`
    : emoteCdnUrl(emote.emoteId, '3x');

  return (
    <div className="group relative glass-panel rounded-lg p-2 flex flex-col items-center">
      {/* Image (click for detail) */}
      <button
        onClick={onOpenDetail}
        className="h-14 w-full flex items-center justify-center cursor-pointer"
      >
        <img
          src={src}
          alt={emote.alias}
          loading="lazy"
          onError={() => !fallback && setFallback(true)}
          className="max-h-14 max-w-full object-contain"
        />
      </button>

      {/* Name / rename */}
      {renaming ? (
        <input
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              onRename(draft);
              setRenaming(false);
            } else if (e.key === 'Escape') {
              setDraft(emote.alias);
              setRenaming(false);
            }
          }}
          onBlur={() => {
            setRenaming(false);
            setDraft(emote.alias);
          }}
          className="mt-1 w-full text-center text-xs bg-glass rounded px-1 py-0.5 text-textPrimary outline-none focus:ring-1 focus:ring-accent/40"
        />
      ) : (
        <div className="mt-1 w-full text-center">
          <div className="text-xs text-textPrimary truncate">
            {emote.alias}
          </div>
          {emote.zeroWidth && (
            <div className="text-[9px] text-accent leading-tight flex items-center justify-center gap-0.5">
              <LayersIcon size={9} /> overlay
            </div>
          )}
        </div>
      )}

      {/* Hover actions */}
      {!renaming && (
        <div className="absolute top-1 right-1 flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
          {confirmRemove ? (
            <>
              <Tooltip content="Confirm remove" side="top" delay={150}>
                <button
                  onClick={onRemove}
                  className="p-1 rounded bg-glass text-red-400 hover:bg-glass-active"
                >
                  <Check size={13} />
                </button>
              </Tooltip>
              <Tooltip content="Cancel" side="top" delay={150}>
                <button
                  onClick={() => setConfirmRemove(false)}
                  className="p-1 rounded bg-glass text-textSecondary hover:bg-glass-active"
                >
                  <X size={13} />
                </button>
              </Tooltip>
            </>
          ) : (
            <>
              <Tooltip content="Copy link" side="top" delay={200}>
                <button
                  onClick={() => {
                    navigator.clipboard?.writeText(`https://7tv.app/emotes/${emote.emoteId}`);
                    addToast('Emote link copied', 'success');
                  }}
                  className="p-1 rounded bg-glass text-textSecondary hover:text-textPrimary hover:bg-glass-active"
                >
                  <Copy size={12} />
                </button>
              </Tooltip>
              <Tooltip content="Open on 7tv.app" side="top" delay={200}>
                <button
                  onClick={() => invoke('open_browser_url', { url: `https://7tv.app/emotes/${emote.emoteId}` }).catch(() => {})}
                  className="p-1 rounded bg-glass text-textSecondary hover:text-textPrimary hover:bg-glass-active"
                >
                  <ExternalLink size={12} />
                </button>
              </Tooltip>
              {canManage && (
                <>
                  <Tooltip content={emote.zeroWidth ? 'Unset overlay' : 'Make overlay (zero-width)'} side="top" delay={200}>
                    <button
                      onClick={onToggleZeroWidth}
                      className={`p-1 rounded bg-glass hover:bg-glass-active ${
                        emote.zeroWidth ? 'text-accent' : 'text-textSecondary hover:text-textPrimary'
                      }`}
                    >
                      <LayersIcon size={12} />
                    </button>
                  </Tooltip>
                  <Tooltip content="Rename" side="top" delay={200}>
                    <button
                      onClick={() => {
                        setDraft(emote.alias);
                        setRenaming(true);
                      }}
                      className="p-1 rounded bg-glass text-textSecondary hover:text-textPrimary hover:bg-glass-active"
                    >
                      <Pencil size={12} />
                    </button>
                  </Tooltip>
                  <Tooltip content="Remove" side="top" delay={200}>
                    <button
                      onClick={() => setConfirmRemove(true)}
                      className="p-1 rounded bg-glass text-textSecondary hover:text-red-400 hover:bg-glass-active"
                    >
                      <Trash2 size={12} />
                    </button>
                  </Tooltip>
                </>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
