import { useEffect, useMemo, useRef, useState } from 'react';
import { Search, X } from 'lucide-react';
import { IS_MOBILE } from '../../utils/platform';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { useChannelSearch, type ChannelItem } from '../multi-nook/channelSearch';
import { ChannelResultRow } from '../multi-nook/ChannelResultRow';
import type { ProviderId } from '../../types/providers';

/** Pinned at module scope so the array identity is stable across renders. */
const TWITCH_ONLY: ProviderId[] = ['twitch'];

/**
 * Inline channel search, reusing the same finder and row that back the
 * MultiChat pickers (live follows first, then Twitch search, with avatars and
 * live dots). Reminders and automation rules pick their chats here. On a phone
 * a tap outside the open search puts it away, as every phone pop-out does.
 */
export function ChannelPicker({
  login,
  label = 'Choose a channel',
  providers,
  onPick,
}: {
  /** The chosen channel, shown on the closed button. */
  login?: string;
  /** The closed button's words while nothing is chosen. */
  label?: string;
  /** The platforms to search; Twitch alone when omitted. */
  providers?: readonly ProviderId[];
  /** `id` is the Twitch room id, empty for a search hit and for every other platform (Rust finds the room at save). */
  onPick: (login: string, id: string, provider: ProviderId) => void;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const exclude = useMemo(() => new Set<string>(), []);
  // Twitch only unless the caller names more: a reminder names a Twitch channel
  // and posts through Twitch chat, so a Kick row picked there would never fire.
  // A rule names the platforms it works on, and its pick says which one it is.
  const providerKey = (providers ?? TWITCH_ONLY).join(',');
  const searched = useMemo(() => providerKey.split(',') as ProviderId[], [providerKey]);
  const {
    searchInput,
    setSearchInput,
    visibleItems,
    highlightIndex,
    setHighlightIndex,
    listRef,
    refreshFollowing,
    reset,
  } = useChannelSearch({ excludeKeys: exclude, providers: searched });

  useEffect(() => {
    if (open) void refreshFollowing();
  }, [open, refreshFollowing]);

  const close = () => {
    reset();
    setOpen(false);
  };
  const onPressCapture = useOutsidePress(open && IS_MOBILE, [rootRef], close);

  const select = (item: ChannelItem) => {
    // Search hits without a resolved id fall back to login==id; don't persist
    // that as a real channel id.
    const provider = (item.provider ?? 'twitch') as ProviderId;
    // Another platform's rows carry no Twitch room id.
    const id = provider !== 'twitch' || (item.source === 'search' && item.id === item.login) ? '' : item.id;
    onPick(item.login, id, provider);
    close();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setHighlightIndex((i) => Math.min(i + 1, Math.max(visibleItems.length - 1, 0)));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setHighlightIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const target = visibleItems[highlightIndex];
      if (target) select(target);
    } else if (e.key === 'Escape') {
      setOpen(false);
    }
  };

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="inline-flex items-center gap-2 glass-input text-textPrimary text-sm px-2.5 py-1.5 hover:bg-white/[0.05] transition-colors"
      >
        <Search size={13} className="text-textMuted" />
        {login ? `#${login}` : label}
      </button>
    );
  }

  return (
    <div ref={rootRef} onPointerDownCapture={onPressCapture} className="rounded-lg border border-borderSubtle bg-background/40 p-2">
      <div className="relative mb-1.5">
        <Search size={13} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-textMuted" />
        <input
          autoFocus
          type="text"
          value={searchInput}
          onChange={(e) => setSearchInput(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="Search channels"
          className="w-full glass-input text-textPrimary text-sm pl-7 pr-7 py-1.5"
          spellCheck={false}
        />
        <button
          type="button"
          onClick={close}
          className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-textMuted hover:bg-white/[0.06] hover:text-textPrimary"
          aria-label="Close"
        >
          <X size={12} />
        </button>
      </div>
      <div ref={listRef} className="max-h-52 overflow-y-auto scrollbar-thin space-y-0.5">
        {visibleItems.map((item, i) => (
          <ChannelResultRow
            key={`${item.login}-${i}`}
            item={item}
            index={i}
            highlighted={highlightIndex === i}
            onSelect={select}
            onHover={setHighlightIndex}
          />
        ))}
        {visibleItems.length === 0 && (
          <div className="px-3 py-3 text-center text-[11px] text-textMuted">
            {searchInput.trim() ? 'No channels found.' : 'Type to search, or pick a live follow.'}
          </div>
        )}
      </div>
    </div>
  );
}

export default ChannelPicker;
