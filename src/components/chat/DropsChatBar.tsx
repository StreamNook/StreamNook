// Drops progress above the message box, for the channel this account is
// earning on. Which reward, how far along and what can be claimed all come
// from Rust (services/watch_drops.rs, through stores/watchDropsStore); this
// draws the one-line bar and, on a click, the drawer listing every reward you
// can earn here. The drawer floats over chat, so nothing in the column moves.
import { lazy, memo, Suspense, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { useWatchDrops, type WatchDrops } from '../../stores/watchDropsStore';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { Logger } from '../../utils/logger';
import { ACCENT_BUTTON, GLAZE_BANNER, SUCCESS_FILL } from '../ui/glazeButtons';
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
    <span className="drops-stack flex shrink-0 items-center pl-1.5">
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

/** The reward's progress as `--drops-heat` (0 to 1), which warms the glass.
 *  The glaze rounds to a pill and outranks the rounded utilities, so each
 *  surface's corner is set inline too; the frost lives in the stylesheet,
 *  where the full-screen column can raise it. */
const heat = (line: BarLine, radius: string | number): CSSProperties =>
  ({ borderRadius: radius, '--drops-heat': (clamp(line.percent) / 100).toFixed(2) }) as CSSProperties;

const clamp = (n: number) => Math.max(0, Math.min(100, n));

/** A drops track: an inset rounded fill, never an edge strip. */
function Track({ line, className }: { line: BarLine; className: string }) {
  return (
    <span aria-hidden className={`block overflow-hidden rounded-full bg-textPrimary/[0.10] ${className}`}>
      <span
        className={`drops-bar-fill block h-full rounded-full ${line.ready ? 'is-ready' : ''}`}
        style={{ width: `${clamp(line.percent)}%` }}
      />
    </span>
  );
}

/** The time strong and the rest quiet ("12m to watch"); `short` keeps only the time. */
function Status({ line, short }: { line: BarLine; short?: boolean }) {
  const { time, rest } = splitStatus(line.status);
  return (
    <span className="shrink-0 whitespace-nowrap tabular-nums text-textSecondary">
      {time && <span className="font-semibold text-textPrimary">{time}</span>}
      {short && time ? '' : rest}
    </span>
  );
}

function ClaimButton({ line, claiming, onClaim }: { line: BarLine; claiming: boolean; onClaim: () => void }) {
  return (
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
  );
}

/** At rest: a slim glazed lip that rises out of the top of the message box,
 *  so nothing floats over chat. The picture, the name, a short track and the
 *  time; a reward ready to claim turns the glass green and says "Ready", and
 *  the Claim itself waits in the hover card and the drawer, so a stray click
 *  on the lip never claims. From props alone, so the settings preview draws
 *  it too. */
export function DropsChatBarView({
  line,
  open,
  onToggle,
  onPreload,
}: Pick<DropsChatBarViewProps, 'line' | 'open' | 'onToggle' | 'onPreload'>) {
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
      style={heat(line, '10px 10px 0 0')}
      className={`${GLAZE_BANNER} drops-bar drops-lip flex h-6 w-full cursor-pointer select-none items-center gap-2 px-2.5 text-[11px] leading-4 outline-none focus-visible:ring-1 focus-visible:ring-accent/60 ${line.ready ? 'is-ready' : ''}`}
    >
      <DropArt image={line.image} size={14} radius={4} />
      <span className="min-w-0 flex-1 truncate font-medium text-textPrimary/90">{line.name}</span>
      <Track line={line} className="h-[3px] w-12 shrink-0" />
      {line.ready ? (
        <span className="shrink-0 whitespace-nowrap font-semibold text-success">Ready</span>
      ) : (
        <Status line={line} short />
      )}
    </div>
  );
}

/** On hover: the lip grows into a card over the bottom of chat, with the
 *  reward, its track and time (or Claim), and the other rewards here. Square
 *  at the bottom like the lip, since it rises out of the same edge of the
 *  message box. A click on it opens the drawer. */
export function DropsPeekView({ line, claiming, onToggle, onClaim }: Omit<DropsChatBarViewProps, 'open' | 'onPreload'>) {
  return (
    <div
      onClick={onToggle}
      style={heat(line, '14px 14px 0 0')}
      className={`${GLAZE_BANNER} drops-bar drops-peek w-full cursor-pointer select-none px-3 pb-2.5 pt-3 ${line.ready ? 'is-ready' : ''}`}
    >
      <div className="flex min-w-0 items-center gap-2.5">
        <DropArt image={line.image} size={32} radius={8} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[12.5px] font-medium leading-4 text-textPrimary">{line.name}</div>
          <div className="mt-2 flex min-w-0 items-center gap-2.5 text-[11.5px] leading-4">
            <Track line={line} className="h-[3px] min-w-0 flex-1" />
            {line.ready && onClaim ? <ClaimButton line={line} claiming={claiming} onClaim={onClaim} /> : <Status line={line} />}
          </div>
        </div>
      </div>
      {line.more > 0 && (
        <div className="mt-2.5 flex min-w-0 items-center gap-2 border-t border-white/[0.07] pt-2 text-[11px] leading-4">
          <MoreStack more={line.more} art={line.moreArt} />
          <span className="min-w-0 flex-1 truncate text-textSecondary">
            {line.more === 1 ? '1 more on this channel' : `${line.more} more on this channel`}
          </span>
          <span className="shrink-0 text-textPrimary/80">Click for all</span>
        </div>
      )}
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
  // The hover card: a mouse resting on the lip (or the card itself) shows it
  // after a beat, so a pointer crossing on its way to the message box does
  // not flash it; leaving gives a short grace to reach the card.
  const [peek, setPeek] = useState(false);
  const peekTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hover = (on: boolean) => (e: PointerEvent<HTMLDivElement>) => {
    if (e.pointerType !== 'mouse') return;
    if (peekTimer.current) clearTimeout(peekTimer.current);
    peekTimer.current = setTimeout(() => setPeek(on), on ? PEEK_SHOW_MS : PEEK_HIDE_MS);
  };
  useEffect(() => () => {
    if (peekTimer.current) clearTimeout(peekTimer.current);
  }, []);

  const send = (event: DrawerEvent) => {
    const next = drawerAfter(open, event);
    if (next && !open) onDrawerOpen();
    if (next) setPeek(false);
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

  // Escape closes the open drawer wherever focus is: opened from the hover
  // card, which goes away as the drawer comes up, focus is left on the page,
  // not in here. Caught first and stopped, so it closes only the drawer.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      setOpen(drawerAfter(true, 'escape'));
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [open]);

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
      onPointerEnter={hover(true)}
      onPointerLeave={hover(false)}
      // The lip sits flush on the message box's top edge (over its border), so
      // at rest it takes only its own height from the column.
      className="relative z-[55] -mb-px flex-shrink-0 px-2 pt-1.5"
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
        {peek && !open && (
          <motion.div
            key="peek"
            initial={{ opacity: 0, y: 6, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 4, scale: 0.985 }}
            transition={{ duration: 0.16, ease: [0.2, 0.8, 0.2, 1] }}
            // Above the lip (which comes later and positions itself), so the
            // card covers it whole and grows out of the same place.
            className="absolute bottom-0 left-2 right-2 z-10 origin-bottom"
          >
            <DropsPeekView
              line={line}
              claiming={!!lead && claiming === lead.drop_id}
              onToggle={() => send('toggle')}
              onClaim={lead ? () => void claim(lead.drop_id, lead.instance_id) : undefined}
            />
          </motion.div>
        )}
      </AnimatePresence>
      <DropsChatBarView line={line} open={open} onToggle={() => send('toggle')} onPreload={() => void loadDrawer()} />
    </div>
  );
}

const PEEK_SHOW_MS = 120;
const PEEK_HIDE_MS = 160;

/** Memoized: ChatWidget re-renders on every chat flush and is not compiled,
 *  and this takes only primitives plus one stable callback. */
const DropsChatBar = memo(DropsChatBarInner);
export default DropsChatBar;
