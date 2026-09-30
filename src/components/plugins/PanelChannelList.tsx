import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { Users, Plus, X, Loader2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { PanelChannel } from '../../types/plugins';
import { Logger } from '../../utils/logger';

interface ChannelResult {
  id?: string;
  user_id?: string;
  user_login?: string;
  broadcaster_login?: string;
  user_name?: string;
  display_name?: string;
  thumbnail_url?: string;
  profile_image_url?: string;
  is_live?: boolean;
  game_name?: string;
}

/**
 * Generic channel-picker field for plugin settings panels: search Twitch as
 * you type, pick from a dropdown of results (avatars, live dots), and manage a
 * removable list. Any plugin that declares a `channel_list` field gets this;
 * it has no knowledge of which plugin or feature it serves.
 */
const PanelChannelList = ({
  value,
  onChange,
}: {
  value: PanelChannel[];
  onChange: (next: PanelChannel[]) => void;
}) => {
  const { followedStreams, loadFollowedStreams } = useAppStore();
  const [input, setInput] = useState('');
  const [results, setResults] = useState<ChannelResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [open, setOpen] = useState(false);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [menuStyle, setMenuStyle] = useState<CSSProperties>({});
  const menuOpen = open && input.trim() !== '';

  useEffect(() => {
    if (followedStreams.length === 0) loadFollowedStreams();
  }, [followedStreams.length, loadFollowedStreams]);

  useEffect(() => {
    const onClickOutside = (e: MouseEvent) => {
      const t = e.target as Node;
      if (boxRef.current?.contains(t) || menuRef.current?.contains(t)) return;
      setOpen(false);
    };
    document.addEventListener('mousedown', onClickOutside);
    return () => document.removeEventListener('mousedown', onClickOutside);
  }, []);

  // The results menu is portalled to <body> with fixed coordinates: the row
  // this field sits in lives inside a settings card that clips its overflow,
  // so an in-place dropdown was cut off at the card's bottom edge. It opens
  // upward when the window has more room above the field than below it.
  useLayoutEffect(() => {
    if (!menuOpen) return;
    const reposition = () => {
      const el = boxRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      const below = window.innerHeight - r.bottom - 16;
      const above = r.top - 16;
      const up = below < 224 && above > below;
      setMenuStyle({
        position: 'fixed',
        left: Math.round(r.left),
        width: Math.round(r.width),
        zIndex: 9999,
        maxHeight: Math.max(120, Math.min(280, up ? above : below)),
        ...(up
          ? { bottom: Math.round(window.innerHeight - r.top + 8) }
          : { top: Math.round(r.bottom + 8) }),
      });
    };
    reposition();
    // Follow the field as the page scrolls; a scroll inside the menu itself
    // leaves it where it is.
    const onScroll = (e: Event) => {
      if (menuRef.current?.contains(e.target as Node)) return;
      reposition();
    };
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', onScroll, true);
    return () => {
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', onScroll, true);
    };
  }, [menuOpen]);

  useEffect(() => {
    if (!input.trim()) {
      setResults([]);
      setSearching(false);
      return;
    }
    if (timeoutRef.current) clearTimeout(timeoutRef.current);
    setSearching(true);
    timeoutRef.current = setTimeout(async () => {
      try {
        const found = (await invoke('search_channels', { query: input })) as ChannelResult[];
        const existing = value.map((c) => c.channel_login);
        const followedIds = new Set(followedStreams.map((s) => s.user_id));
        const filtered = found
          .filter((r) => !existing.includes(r.user_login || r.broadcaster_login || ''))
          .sort((a, b) => {
            const aF = followedIds.has(a.user_id || a.id || '');
            const bF = followedIds.has(b.user_id || b.id || '');
            if (aF !== bF) return aF ? -1 : 1;
            if (!!a.is_live !== !!b.is_live) return a.is_live ? -1 : 1;
            return 0;
          });
        setResults(filtered.slice(0, 5));
      } catch (err) {
        Logger.error('[PanelChannelList] search failed:', err);
        setResults([]);
      } finally {
        setSearching(false);
      }
    }, 500);
    return () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, [input, value, followedStreams]);

  const add = (r: ChannelResult) => {
    const id = r.user_id || r.id || '';
    const login = r.user_login || r.broadcaster_login || '';
    if (!id || value.some((c) => c.channel_id === id)) return;
    onChange([
      ...value,
      { channel_id: id, channel_login: login, display_name: r.user_name || r.display_name || login },
    ]);
    setInput('');
    setOpen(false);
  };

  const remove = (id: string) => onChange(value.filter((c) => c.channel_id !== id));

  return (
    <div className="mt-2">
      <div className="mb-2 space-y-1.5">
        {value.length > 0 ? (
          value.map((c, i) => (
            <Row key={c.channel_id} channel={c} index={i} followedStreams={followedStreams} onRemove={() => remove(c.channel_id)} />
          ))
        ) : (
          <div className="rounded-lg border border-dashed border-white/10 bg-white/[0.02] px-3 py-2.5 text-center text-[12px] italic text-textSecondary">
            None added.
          </div>
        )}
      </div>
      <div className="relative" ref={boxRef}>
        <input
          type="text"
          value={input}
          placeholder="Search for a channel..."
          onChange={(e) => {
            setInput(e.target.value);
            setOpen(true);
          }}
          onFocus={() => input.trim() && setOpen(true)}
          className="glass-input w-full rounded-md px-3 py-1.5 text-[13px] text-textPrimary"
        />
        {searching && (
          <div className="absolute right-3 top-0 bottom-0 flex items-center">
            <Loader2 size={15} className="animate-spin text-accent" />
          </div>
        )}
        {menuOpen && createPortal(
          <div ref={menuRef} style={menuStyle} className="glass-panel flex flex-col overflow-hidden">
            {searching && results.length === 0 ? (
              <div className="p-3 text-center text-[12px] italic text-textSecondary">Searching...</div>
            ) : results.length > 0 ? (
              <div className="min-h-0 flex-1 overflow-y-auto custom-scrollbar">
                {results.map((r) => {
                  const login = r.user_login || r.broadcaster_login || '';
                  const name = r.user_name || r.display_name || login;
                  const avatar = r.profile_image_url || r.thumbnail_url;
                  return (
                    <button
                      key={r.user_id || r.id}
                      type="button"
                      onClick={() => add(r)}
                      className="group flex w-full items-center gap-3 px-3 py-2.5 text-left transition-colors hover:bg-white/[0.06]"
                    >
                      {avatar ? (
                        <img src={avatar} alt={name} className="h-8 w-8 rounded-full object-cover" />
                      ) : (
                        <div className="flex h-8 w-8 items-center justify-center rounded-full bg-white/10">
                          <Users size={14} className="text-textSecondary" />
                        </div>
                      )}
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-1.5">
                          <span className="truncate text-[13px] font-medium text-textPrimary">{name}</span>
                          {r.is_live && <span className="h-1.5 w-1.5 rounded-full bg-red-500" />}
                        </div>
                        <div className="truncate text-[11px] text-textSecondary">{r.game_name || login}</div>
                      </div>
                      <Plus size={15} className="flex-shrink-0 text-textSecondary opacity-0 transition-opacity group-hover:opacity-100" />
                    </button>
                  );
                })}
              </div>
            ) : (
              <div className="p-3 text-center text-[12px] italic text-textSecondary">No channels found.</div>
            )}
          </div>,
          document.body,
        )}
      </div>
    </div>
  );
};

const Row = ({
  channel,
  index,
  followedStreams,
  onRemove,
}: {
  channel: PanelChannel;
  index: number;
  followedStreams: Array<{ user_id: string; profile_image_url?: string }>;
  onRemove: () => void;
}) => {
  const followed = followedStreams.find((s) => s.user_id === channel.channel_id);
  const [avatar, setAvatar] = useState<string | null>(followed?.profile_image_url ?? null);
  useEffect(() => {
    if (avatar) return;
    let mounted = true;
    invoke<{ profile_image_url?: string }>('get_user_by_id', { userId: channel.channel_id })
      .then((info) => {
        if (mounted && info?.profile_image_url) setAvatar(info.profile_image_url);
      })
      .catch(() => {});
    return () => {
      mounted = false;
    };
  }, [channel.channel_id, avatar]);
  return (
    <div className="group flex items-center gap-3 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2">
      <span className="flex h-5 w-5 flex-shrink-0 items-center justify-center rounded bg-white/5 font-mono text-[11px] text-textSecondary">
        {index + 1}
      </span>
      {avatar ? (
        <img src={avatar} alt={channel.display_name} className="h-7 w-7 flex-shrink-0 rounded-full object-cover" />
      ) : (
        <div className="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-full bg-white/10">
          <Users size={13} className="text-textSecondary" />
        </div>
      )}
      <div className="min-w-0 flex-1">
        <span className="truncate text-[13px] font-medium text-textPrimary">{channel.display_name}</span>
        {followed && <span className="ml-1.5 inline-block h-1.5 w-1.5 rounded-full bg-red-500 align-middle" />}
      </div>
      <button
        type="button"
        onClick={onRemove}
        className="rounded p-1 text-textMuted opacity-0 transition-all hover:bg-red-500/10 hover:text-red-300 group-hover:opacity-100"
      >
        <X size={14} />
      </button>
    </div>
  );
};

export default PanelChannelList;
