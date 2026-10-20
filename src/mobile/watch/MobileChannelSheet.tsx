// A channel's videos and clips, the iPhone's Channel sheet on Android: from the
// player's avatar and the offline room's Videos. Two tabs: Videos (past
// broadcasts, highlights or uploads, recent or popular) and Clips (by period).
// Rust `channel_media` words every row. A video plays in the watch layer (the
// shared `playMedia` start, with its chat replay); a clip opens the clip sheet.
// Holding a row offers Copy link, and on a clip "Watch full broadcast from here".
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { CaretDown, Check, LinkSimple, PlayCircle } from 'phosphor-react';
import type { TwitchStream } from '../../types';
import { useAppStore } from '../../stores/AppStore';
import { MobileSheet } from '../ui/MobileSheet';
import { SegmentedControl, type Segment } from '../ui/SegmentedControl';
import { EmptyNote } from '../ui/EmptyNote';
import { hapticCommit } from '../ui/haptics';
import { useMobileNavStore } from '../navStore';
import PenroseMarch from '../../components/PenroseMarch';
import { Logger } from '../../utils/logger';
import {
  CLIP_PERIODS,
  VIDEO_KINDS,
  VIDEO_SORTS,
  countLine,
  emptyLine,
  mergePage,
  metaLine,
  shelfArgs,
  shelfKey,
  type ClipPeriod,
  type MediaPage,
  type MediaRow,
  type MediaShelf,
  type MediaTab,
  type VideoKind,
  type VideoSort,
} from './channelMedia';

interface ChannelSheetState {
  channel: TwitchStream | null;
  open: (channel: TwitchStream) => void;
  close: () => void;
}

/** Which channel's sheet is open. One at a time, over everything. */
export const useChannelSheet = create<ChannelSheetState>((set) => ({
  channel: null,
  open: (channel) => set({ channel }),
  close: () => set({ channel: null }),
}));

const TABS: Segment<MediaTab>[] = [
  { value: 'videos', label: 'Videos' },
  { value: 'clips', label: 'Clips' },
];

/** How long a press must rest before it is a hold, and how far a finger may travel. */
const HOLD_MS = 450;
const SLOP_PX = 10;

/** Plays one of a channel's videos in the watch layer, full screen. `at` is a clip's
 *  moment in it, which wins over the saved position. */
export function openChannelVod(channel: TwitchStream, row: { id: string; title: string; thumbnailUrl: string }, at?: number) {
  useChannelSheet.getState().close();
  useMobileNavStore.getState().setPlayerMode('full');
  void useAppStore.getState().playMedia('video', `https://www.twitch.tv/videos/${row.id}`, {
    id: row.id,
    user_id: channel.user_id,
    user_login: channel.user_login,
    user_name: channel.user_name,
    title: row.title,
    thumbnail_url: row.thumbnailUrl,
    start_at: at,
  });
}

export const MobileChannelSheet: React.FC = () => {
  const channel = useChannelSheet((s) => s.channel);
  const close = useChannelSheet((s) => s.close);
  return (
    <MobileSheet open={!!channel} onClose={close} maxHeightFraction={0.92} fill>
      {channel && <ChannelMedia key={channel.user_id || channel.user_login} channel={channel} />}
    </MobileSheet>
  );
};

const ChannelMedia: React.FC<{ channel: TwitchStream }> = ({ channel }) => {
  const [tab, setTab] = useState<MediaTab>('videos');
  const [kind, setKind] = useState<VideoKind>('archive');
  const [sort, setSort] = useState<VideoSort>('recent');
  const [period, setPeriod] = useState<ClipPeriod>('30d');
  const [sortOpen, setSortOpen] = useState(false);
  const [rows, setRows] = useState<MediaRow[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [held, setHeld] = useState<MediaRow | null>(null);
  const shelf: MediaShelf = tab === 'videos' ? { tab, kind, sort } : { tab, period };
  // The shelf a page was asked for; an answer for another one is dropped.
  const asked = useRef(shelfKey(shelf));
  const busy = useRef(false);

  const isLive = useAppStore(
    (s) =>
      s.currentMediaType === 'live' &&
      !!s.currentStream &&
      s.currentStream.user_login.toLowerCase() === channel.user_login.toLowerCase(),
  );

  const load = useCallback(
    async (reset: boolean, from: string | null) => {
      const shelf: MediaShelf = tab === 'videos' ? { tab, kind, sort } : { tab, period };
      const want = shelfKey(shelf);
      if (!reset && busy.current) return;
      if (!channel.user_id) {
        setFailed(true);
        return;
      }
      asked.current = want;
      busy.current = true;
      setLoading(true);
      setFailed(false);
      if (reset) {
        setRows([]);
        setCursor(null);
      }
      try {
        const page = await invoke<MediaPage>('channel_media', shelfArgs(shelf, channel.user_id, reset ? null : from));
        if (asked.current !== want) return;
        setRows((shown) => mergePage(reset ? [] : shown, page.rows));
        setCursor(page.cursor);
      } catch (e) {
        if (asked.current !== want) return;
        Logger.warn('[ChannelSheet] channel_media failed:', e);
        setFailed(true);
      } finally {
        if (asked.current === want) {
          busy.current = false;
          setLoading(false);
        }
      }
    },
    [tab, kind, sort, period, channel.user_id],
  );

  useEffect(() => {
    void load(true, null);
  }, [load]);

  // The next page when the last row comes into view.
  const tailRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const tail = tailRef.current;
    if (!tail || !cursor) return;
    const seen = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) void load(false, cursor);
    });
    seen.observe(tail);
    return () => seen.disconnect();
  }, [cursor, load]);

  const open = (row: MediaRow) => {
    if (row.kind === 'clip') {
      useChannelSheet.getState().close();
      useAppStore.getState().openClipModal(row.url, {
        id: row.id,
        title: row.title,
        thumbnail_url: row.thumbnailUrl,
        broadcaster_id: channel.user_id,
        broadcaster_name: channel.user_name,
      });
    } else {
      openChannelVod(channel, row);
    }
  };

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-3 px-1 pb-3.5 pt-1">
        {channel.profile_image_url ? (
          <img src={channel.profile_image_url} alt="" className="h-11 w-11 shrink-0 rounded-full object-cover" />
        ) : (
          <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full bg-white/10 text-base font-bold text-textPrimary">
            {(channel.user_name || channel.user_login).charAt(0).toUpperCase()}
          </div>
        )}
        <div className="min-w-0 flex-1">
          <div className="truncate text-[20px] font-bold leading-6 text-textPrimary">
            {channel.user_name || channel.user_login}
          </div>
          <div className="text-[13px] leading-[18px] text-textSecondary">{isLive ? 'Live now' : 'Offline'}</div>
        </div>
      </div>

      <SegmentedControl label="Show" segments={TABS} value={tab} onChange={setTab} fill />

      {tab === 'videos' ? (
        <div className="flex flex-col items-start gap-1.5 pt-3">
          <SegmentedControl label="Kind" segments={VIDEO_KINDS} value={kind} onChange={setKind} />
          <button
            type="button"
            onClick={() => setSortOpen(true)}
            className="flex min-h-8 items-center gap-1 text-[13px] font-medium text-textSecondary"
          >
            {VIDEO_SORTS.find((s) => s.value === sort)?.label}
            <CaretDown size={10} weight="bold" />
          </button>
        </div>
      ) : (
        <div className="pb-2 pt-3">
          <SegmentedControl label="Period" segments={CLIP_PERIODS} value={period} onChange={setPeriod} />
        </div>
      )}

      <div className="flex flex-col gap-3.5 py-2">
        {rows.map((row) => (
          <MediaRowButton key={row.id} row={row} onOpen={open} onHold={setHeld} />
        ))}
        <div ref={tailRef} aria-hidden className="h-px" />
        {loading ? (
          <div className="flex justify-center py-4">
            <PenroseMarch size={28} />
          </div>
        ) : failed ? (
          <div className="flex flex-col items-center">
            <EmptyNote text="Could not load these" hint={null} />
            <button
              type="button"
              onClick={() => void load(true, null)}
              className="-mt-14 min-h-11 px-4 text-[14px] font-semibold text-accent"
            >
              Try again
            </button>
          </div>
        ) : rows.length === 0 ? (
          <EmptyNote text={emptyLine(shelf)} hint={null} />
        ) : null}
      </div>

      <MobileSheet open={sortOpen} onClose={() => setSortOpen(false)} title="Sort">
        <div className="flex flex-col">
          {VIDEO_SORTS.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => {
                setSort(option.value);
                setSortOpen(false);
              }}
              className="flex min-h-12 items-center justify-between text-[15px] text-textPrimary"
            >
              {option.label}
              {option.value === sort && <Check size={16} weight="bold" className="text-accent" />}
            </button>
          ))}
        </div>
      </MobileSheet>

      <MobileSheet open={!!held} onClose={() => setHeld(null)} title={held?.title}>
        <div className="flex flex-col">
          <button
            type="button"
            onClick={() => {
              if (held) void navigator.clipboard?.writeText(held.url).catch(() => {});
              useAppStore.getState().addToast('Link copied', 'success');
              setHeld(null);
            }}
            className="flex min-h-12 items-center gap-3 text-[15px] text-textPrimary"
          >
            <LinkSimple size={18} />
            Copy link
          </button>
          {held?.kind === 'clip' && held.vodId && (
            <button
              type="button"
              onClick={() => {
                const row = held;
                setHeld(null);
                if (row.vodId) {
                  openChannelVod(channel, { id: row.vodId, title: row.title, thumbnailUrl: row.thumbnailUrl }, row.vodOffset ?? undefined);
                }
              }}
              className="flex min-h-12 items-center gap-3 text-[15px] text-textPrimary"
            >
              <PlayCircle size={18} />
              Watch full broadcast from here
            </button>
          )}
        </div>
      </MobileSheet>
    </div>
  );
};

/** A video or clip row: the thumbnail with no words on it, the resume bar under it
 *  when there is one, then the title and up to two quiet lines. A tap opens it, a
 *  hold offers the row's menu. */
const MediaRowButton: React.FC<{
  row: MediaRow;
  onOpen: (row: MediaRow) => void;
  onHold: (row: MediaRow) => void;
}> = ({ row, onOpen, onHold }) => {
  const press = useRef<{ x: number; y: number; timer: ReturnType<typeof setTimeout> } | null>(null);
  const heldNow = useRef(false);
  const clear = () => {
    if (press.current) clearTimeout(press.current.timer);
    press.current = null;
  };
  const lines = [metaLine(row), countLine(row)].filter(Boolean);
  return (
    <button
      type="button"
      className="flex w-full items-start gap-3 text-left active:opacity-70 select-none [-webkit-touch-callout:none]"
      onContextMenu={(e) => e.preventDefault()}
      onPointerDown={(e) => {
        heldNow.current = false;
        clear();
        press.current = {
          x: e.clientX,
          y: e.clientY,
          timer: setTimeout(() => {
            press.current = null;
            heldNow.current = true;
            hapticCommit();
            onHold(row);
          }, HOLD_MS),
        };
      }}
      onPointerMove={(e) => {
        const p = press.current;
        if (p && Math.hypot(e.clientX - p.x, e.clientY - p.y) > SLOP_PX) clear();
      }}
      onPointerUp={clear}
      onPointerCancel={clear}
      onClick={() => {
        if (heldNow.current) return;
        onOpen(row);
      }}
    >
      <div className="flex w-36 shrink-0 flex-col gap-[5px]">
        <div className={`aspect-video w-full overflow-hidden rounded-lg bg-surface ${row.watched ? 'opacity-60' : ''}`}>
          {row.thumbnailUrl && (
            <img src={row.thumbnailUrl} alt="" loading="lazy" draggable={false} className="h-full w-full object-cover" />
          )}
        </div>
        {row.progress != null && (
          <div className="h-[3px] w-full overflow-hidden rounded-full bg-white/[0.12]">
            <div
              className={`h-full rounded-full ${row.watched ? 'bg-white/40' : 'bg-accent'}`}
              style={{ width: `${Math.max(4, Math.min(100, row.progress * 100))}%` }}
            />
          </div>
        )}
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="line-clamp-2 text-[14px] font-semibold leading-[18px] text-textPrimary">{row.title}</div>
        {lines.map((line, i) => (
          <div key={i} className="truncate text-[12.5px] leading-4 text-textSecondary">
            {line}
          </div>
        ))}
      </div>
    </button>
  );
};
