// Drops progress above the message box, for the channel this account is
// earning on. Which reward, how far along and what can be claimed all come
// from Rust (services/watch_drops.rs, through stores/watchDropsStore); this
// draws the one-line bar and, on a click, the drawer listing every reward you
// can earn here. The drawer floats over chat, so nothing in the column moves.
import { lazy, memo, Suspense, useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { AnimatePresence } from 'framer-motion';
import { CaretUp } from 'phosphor-react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { useWatchDrops, type WatchDrops } from '../../stores/watchDropsStore';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { Logger } from '../../utils/logger';
import { ACCENT_BUTTON, SUCCESS_FILL } from '../ui/glazeButtons';
import { DropArt } from './DropArt';
import { splitStatus } from './dropsStatusText';

// The drawer's code loads on first open (or first hover of the bar), so a chat
// that never opens it pays nothing for it.
const loadDrawer = () => import('./DropsDrawer');
const DropsDrawer = lazy(loadDrawer);

/** This chat is the one earning: ids when both sides know theirs (a pane can
 *  be waiting on its id), else logins, ignoring case. Bare Twitch identity
 *  only; the caller mounts the bar on Twitch chats alone. */
export function showsHere(view: WatchDrops, channelId: string, login: string): boolean {
  if (!view.channel_id && !view.login) return false;
  if (channelId && view.channel_id) return channelId === view.channel_id;
  return !!login && !!view.login && login.toLowerCase() === view.login.toLowerCase();
}

/** What the bar's one line says. */
export interface BarLine {
  name: string;
  image: string;
  status: string;
  /** 0 to 100. */
  percent: number;
  /** Waits on a claim: the status becomes the Claim button. */
  ready: boolean;
  /** Other rewards under way here. */
  more: number;
  /** Pictures of the first two of them, stacked; any beyond read "+N". */
  moreArt: string[];
  spoken: string;
  claimLabel: string;
}

/** The lead reward; with none under way yet, the first reward a campaign here
 *  will start on; with nothing left to earn here, no bar. */
export function barLine(view: WatchDrops): BarLine | null {
  const lead = view.lead;
  if (lead) {
    return {
      name: lead.name,
      image: lead.image,
      status: lead.status,
      percent: lead.percent,
      ready: lead.complete,
      more: view.more,
      moreArt: view.more_art ?? [],
      spoken: lead.spoken,
      claimLabel: lead.claim_label,
    };
  }
  const waiting = view.campaigns.filter((c) => !c.started && c.drops.length > 0);
  if (waiting.length === 0) return null;
  const first = waiting[0].drops[0];
  const rewards = waiting.reduce((n, c) => n + c.drops.length, 0);
  return {
    name: rewards === 1 ? first.name : `${rewards} rewards to earn here`,
    image: rewards === 1 ? first.image : waiting[0].art,
    status: first.status,
    percent: 0,
    ready: false,
    more: 0,
    moreArt: [],
    spoken: rewards === 1 ? `${first.name}, ${first.status}` : `${rewards} rewards to earn on this channel`,
    claimLabel: '',
  };
}

/** The settings preview's sample line. */
export const PREVIEW_LINE: BarLine = {
  name: 'Rust hoodie',
  image: '',
  status: '12m to watch',
  percent: 72,
  ready: false,
  more: 2,
  moreArt: [],
  spoken: 'Rust hoodie, 12 minutes left',
  claimLabel: 'Claim',
};

export type DrawerEvent = 'toggle' | 'outside' | 'escape' | 'picker' | 'gone';

/** Every way the drawer changes: only the bar's own click opens it, and
 *  everything else (a press outside, Escape, the emote picker opening, the bar
 *  going away) closes it. */
export const drawerAfter = (open: boolean, event: DrawerEvent): boolean => (event === 'toggle' ? !open : false);

/** Claims one drop. "claimed" and "already_claimed" both mean it is yours, so
 *  both say so; the toast comes only from this click. */
function useDropClaim() {
  const [claiming, setClaiming] = useState<string | null>(null);
  const claim = async (dropId: string, instanceId: string | null) => {
    if (claiming) return;
    setClaiming(dropId);
    try {
      await invoke<string>('claim_drop', { dropId, dropInstanceId: instanceId });
      useAppStore.getState().addToast('Drop claimed', 'success');
    } catch (err) {
      Logger.error('[DropsChatBar] claim failed:', err);
      useAppStore.getState().addToast('Could not claim that drop.', 'error');
    } finally {
      setClaiming(null);
    }
  };
  return { claiming, claim };
}

export interface DropsChatBarViewProps {
  line: BarLine;
  open: boolean;
  claiming: boolean;
  onToggle?: () => void;
  onClaim?: () => void;
  onPreload?: () => void;
}

/** The other rewards on the bar: up to two pictures, then "+N" for the rest;
 *  with no pictures at all, "+N" alone. */
function MoreStack({ more, art }: { more: number; art: string[] }) {
  if (more <= 0) return null;
  const shown = art.slice(0, Math.min(2, more));
  const rest = more - shown.length;
  return (
    <span className="drops-stack flex shrink-0 items-center pl-1">
      {shown.map((src) => (
        <DropArt key={src} image={src} size={18} radius={5} />
      ))}
      {rest > 0 && (
        <span className="rounded-full bg-textPrimary/[0.10] px-1.5 text-[10.5px] font-medium leading-[18px] tabular-nums text-textSecondary">
          +{rest}
        </span>
      )}
    </span>
  );
}

/** The bar itself, from props alone (the settings preview draws it too). The
 *  fill is an inset track under the name, clear of the bar's rounded corners. */
export function DropsChatBarView({ line, open, claiming, onToggle, onClaim, onPreload }: DropsChatBarViewProps) {
  const percent = Math.max(0, Math.min(100, line.percent));
  const { time, rest } = splitStatus(line.status);
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      onToggle?.();
    }
  };
  return (
    <div
      role="button"
      tabIndex={0}
      aria-expanded={open}
      aria-label={line.spoken}
      onClick={onToggle}
      onKeyDown={onKeyDown}
      onPointerEnter={onPreload}
      className={`liquid-glass-panel drops-bar flex h-[42px] w-full cursor-pointer select-none flex-col justify-center gap-[5px] px-2.5 outline-none focus-visible:ring-1 focus-visible:ring-accent/60 ${line.ready ? 'is-ready' : ''}`}
    >
      <span className="flex min-w-0 items-center gap-2">
        <DropArt image={line.image} size={26} radius={7} />
        <span className="min-w-0 flex-1 truncate text-[12.5px] font-medium leading-4 text-textPrimary">{line.name}</span>
        {line.ready && onClaim ? (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onClaim();
            }}
            disabled={claiming}
            style={SUCCESS_FILL}
            className={`${ACCENT_BUTTON} min-w-[60px] shrink-0 px-3 text-center !text-[11.5px] leading-[22px]`}
          >
            {claiming ? 'Claiming' : line.claimLabel}
          </button>
        ) : (
          <span className="shrink-0 whitespace-nowrap text-[12px] leading-4 tabular-nums text-textSecondary">
            {time && <span className="font-semibold text-textPrimary">{time}</span>}
            {rest}
          </span>
        )}
        <MoreStack more={line.more} art={line.moreArt} />
        <CaretUp
          size={12}
          weight="bold"
          aria-hidden
          className={`shrink-0 text-textMuted transition-transform duration-200 ease-[cubic-bezier(0.2,0.8,0.2,1)] ${open ? 'rotate-180' : ''}`}
        />
      </span>
      <span aria-hidden className="ml-[34px] block h-[3px] overflow-hidden rounded-full bg-textPrimary/[0.09]">
        <span className={`drops-bar-fill block h-full rounded-full ${line.ready ? 'is-ready' : ''}`} style={{ width: `${percent}%` }} />
      </span>
    </div>
  );
}

interface DropsChatBarProps {
  /** This chat's Twitch channel id ('' while a pane is still looking it up). */
  channelId: string;
  login: string;
  /** Height of the floating chat header over the column, which the drawer must not reach under. */
  headerInset: number;
  emotePickerOpen: boolean;
  /** Opening the drawer closes the emote picker. Must be stable. */
  onDrawerOpen: () => void;
}

function DropsChatBarInner({ channelId, login, headerInset, emotePickerOpen, onDrawerOpen }: DropsChatBarProps) {
  const enabled = useAppStore((s) => s.settings.chat_design?.show_drops_in_chat ?? true);
  const snapshot = useWatchDrops(enabled);
  const view = snapshot?.view ?? null;
  const line = view && showsHere(view, channelId, login) ? barLine(view) : null;
  const [open, setOpen] = useState(false);
  const [pickerWas, setPickerWas] = useState(emotePickerOpen);
  const [maxHeight, setMaxHeight] = useState(320);
  const { claiming, claim } = useDropClaim();
  const hostRef = useRef<HTMLDivElement>(null);

  const send = (event: DrawerEvent) => {
    const next = drawerAfter(open, event);
    if (next && !open) onDrawerOpen();
    setOpen(next);
  };

  // Adjusted while rendering rather than in an effect: the emote picker
  // opening closes the drawer, and a bar that went away (nothing left here,
  // another channel, the setting off) takes its drawer with it, so it never
  // comes back already open.
  if (pickerWas !== emotePickerOpen) {
    setPickerWas(emotePickerOpen);
    if (emotePickerOpen && open) setOpen(drawerAfter(open, 'picker'));
  }
  if (!line && open) setOpen(drawerAfter(open, 'gone'));

  const onPress = useOutsidePress(open, [hostRef], () => send('outside'));

  // The drawer may rise as far as the header's lower edge. ResizeObserver
  // reports once on observe, so the cap is set as the drawer opens, and again
  // whenever the column resizes.
  useEffect(() => {
    const host = hostRef.current;
    const column = host?.offsetParent;
    if (!open || !host || !column) return;
    const ro = new ResizeObserver(() => setMaxHeight(Math.max(140, host.offsetTop - headerInset - 12)));
    ro.observe(column);
    // A strip above the bar coming or going (the AutoMod queue) moves the bar
    // without resizing the column; the message list beside it resizes instead.
    for (const el of Array.from(column.children)) if (el !== host) ro.observe(el);
    return () => ro.disconnect();
  }, [open, headerInset]);

  if (!line || !view) return null;
  const lead = view.lead;
  return (
    <div
      ref={hostRef}
      onPointerDownCapture={onPress}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && open) {
          e.stopPropagation();
          send('escape');
        }
      }}
      className="relative z-[55] flex-shrink-0 px-2 pb-1 pt-1.5"
    >
      <AnimatePresence>
        {open && (
          <Suspense key="drawer" fallback={null}>
            <DropsDrawer
              view={view}
              maxHeight={maxHeight}
              claimingId={claiming}
              onClaim={(dropId, instanceId) => void claim(dropId, instanceId)}
            />
          </Suspense>
        )}
      </AnimatePresence>
      <DropsChatBarView
        line={line}
        open={open}
        claiming={!!lead && claiming === lead.drop_id}
        onToggle={() => send('toggle')}
        onClaim={lead ? () => void claim(lead.drop_id, lead.instance_id) : undefined}
        onPreload={() => void loadDrawer()}
      />
    </div>
  );
}

/** Memoized: ChatWidget re-renders on every chat flush and is not compiled,
 *  and this takes only primitives plus one stable callback. */
const DropsChatBar = memo(DropsChatBarInner);
export default DropsChatBar;
