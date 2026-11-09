// Drops progress above the message box, for the channel this account is
// earning on. Which reward, how far along and what can be claimed all come
// from Rust (services/watch_drops.rs, through stores/watchDropsStore); this
// draws the one-line bar and, on a click, the drawer listing every reward you
// can earn here. The drawer floats over chat, so nothing in the column moves.
import { lazy, memo, Suspense, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { CaretDown, CaretUp } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import { Tooltip } from '../ui/Tooltip';
import { useWatchDrops, type WatchDrops } from '../../stores/watchDropsStore';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { usePerformanceQuiet } from '../../utils/lowPower';
import { ACCENT_BUTTON, GLAZE_BANNER, SUCCESS_FILL } from '../ui/glazeButtons';
import { DropArt } from './DropArt';
import { ConnectButton } from './DropsConnectButton';
import { useDropClaim } from './useDropClaim';
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
  /** The lead waits on a game account link: where to link it, for the hover
   *  card's Connect. Null otherwise. */
  connectUrl: string | null;
}

/** The lead reward; with none under way yet, the first reward a campaign here
 *  will start on; with nothing left to earn here, no bar. */
export function barLine(view: WatchDrops): BarLine | null {
  const lead = view.lead;
  if (lead) {
    const campaign = view.campaigns.find((c) => c.id === lead.campaign_id);
    const row = campaign?.drops.find((d) => d.lead);
    return {
      connectUrl: row?.connect && !row.claimed ? (campaign?.link_url ?? null) : null,
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
  // Campaigns here not started yet, by the rewards still to earn: one already
  // owned (Twitch reissued the campaign around a reward you hold) is nothing to
  // earn, so a campaign of owned rewards never brings the lip up.
  const waiting = view.campaigns
    .filter((c) => !c.started)
    .map((c) => ({ art: c.art, drops: c.drops.filter((d) => !d.claimed) }))
    .filter((c) => c.drops.length > 0);
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
    connectUrl: null,
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
  connectUrl: null,
  spoken: 'Rust hoodie, 12 minutes left',
  claimLabel: 'Claim',
};

export type DrawerEvent = 'toggle' | 'outside' | 'escape' | 'picker' | 'gone';

/** Every way the drawer changes: only the bar's own click opens it, and
 *  everything else (a press outside, Escape, the emote picker opening, the bar
 *  going away) closes it. */
export const drawerAfter = (open: boolean, event: DrawerEvent): boolean => (event === 'toggle' ? !open : false);


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
  touch,
}: Pick<DropsChatBarViewProps, 'line' | 'open' | 'onToggle' | 'onPreload'> & {
  /** The phones': tall enough for a thumb, and a tap opens the sheet. */
  touch?: boolean;
}) {
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
      style={heat(line, touch ? '12px 12px 0 0' : '10px 10px 0 0')}
      // An arrow, not a hand: a mouse resting here grows the card, which is
      // what takes the click (a tap opens the drawer straight away).
      className={`${GLAZE_BANNER} drops-bar drops-lip flex w-full cursor-default select-none items-center outline-none focus-visible:ring-1 focus-visible:ring-accent/60 ${
        touch ? 'h-9 gap-2.5 px-3 text-[12.5px] leading-4 active:opacity-80' : 'h-6 gap-2 px-2.5 text-[11px] leading-4'
      } ${line.ready ? 'is-ready' : ''}`}
    >
      <DropArt image={line.image} size={touch ? 20 : 14} radius={touch ? 5 : 4} />
      <span className="min-w-0 flex-1 truncate font-medium text-textPrimary/90">{line.name}</span>
      <Track line={line} className={`h-[3px] shrink-0 ${touch ? 'w-16' : 'w-12'}`} />
      {line.ready ? (
        <span className="shrink-0 whitespace-nowrap font-semibold text-success">Ready</span>
      ) : splitStatus(line.status).time ? (
        // Still earning: how far along, then the time left, quieter.
        <span className="shrink-0 whitespace-nowrap tabular-nums">
          <span className="font-semibold text-textPrimary">{Math.round(clamp(line.percent))}%</span>
          <span className="text-textSecondary"> · {splitStatus(line.status).time}</span>
        </span>
      ) : (
        <Status line={line} short />
      )}
    </div>
  );
}

// The tip: a bright point of the fill's own color with a soft halo, quiet
// enough that it never pulls the eye off chat.
const HAIRLINE_TIP: CSSProperties = {
  background: 'color-mix(in srgb, var(--color-accent-on-surface) 70%, white)',
  boxShadow:
    '0 0 4px 1px color-mix(in srgb, var(--color-accent-on-surface) 70%, transparent), 0 0 10px 2px color-mix(in srgb, var(--color-accent-on-surface) 30%, transparent)',
};

/** The lip tucked away: the message box's own top border fills with the
 *  reward's progress, edge to edge, so nothing floats over chat. A strip just
 *  above the border takes the pointer: hovering it shows the card (desktop),
 *  clicking or tapping it brings the lip back. */
export function DropsHairline({
  line,
  onShow,
  onPreload,
  touch,
}: {
  line: BarLine;
  onShow: () => void;
  onPreload?: () => void;
  touch?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onShow}
      onPointerEnter={onPreload}
      aria-label={`${line.spoken}. Show the drops bar`}
      className={`drops-hairline group relative block w-full cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-accent/60 ${touch ? 'h-4' : 'h-2'}`}
    >
      {/* Over the message box's top border (the host overlaps it by a
          pixel): the fill runs from the left edge and thickens a touch under
          the pointer. */}
      <span aria-hidden className="absolute inset-x-0 bottom-0 h-[2px] transition-[height] duration-150 group-hover:h-[3px]">
        <span className="drops-bar-fill block h-full rounded-full" style={{ width: `${clamp(line.percent)}%` }} />
        {/* A soft glow on the leading tip, where the progress is now. */}
        <span
          className="drops-hairline-tip absolute top-1/2 h-1.5 w-1.5 -translate-x-1/2 -translate-y-1/2 rounded-full"
          style={{ left: `${clamp(line.percent)}%`, ...HAIRLINE_TIP }}
        />
      </span>
    </button>
  );
}

/** On hover: the lip grows into a card over the bottom of chat, with the
 *  reward, its track and time (or Claim), and the other rewards here. Square
 *  at the bottom like the lip, since it rises out of the same edge of the
 *  message box. A click on it opens the drawer. */
export function DropsPeekView({
  line,
  claiming,
  onToggle,
  onClaim,
  tucked,
  onTuck,
}: Omit<DropsChatBarViewProps, 'open' | 'onPreload'> & {
  /** The lip is tucked away into the thin line. */
  tucked?: boolean;
  /** Tuck the lip away, or bring it back when it is tucked. */
  onTuck?: () => void;
}) {
  return (
    <div
      onClick={onToggle}
      style={heat(line, '14px 14px 0 0')}
      className={`${GLAZE_BANNER} drops-bar drops-peek relative w-full cursor-pointer select-none px-3 pb-2.5 pt-3 ${line.ready ? 'is-ready' : ''}`}
    >
      {onTuck && (
        <Tooltip content={tucked ? 'Show the drops bar' : 'Tuck away until something new'} side="top">
          <button
            type="button"
            aria-label={tucked ? 'Show the drops bar' : 'Tuck the drops bar away'}
            onClick={(e) => {
              e.stopPropagation();
              onTuck();
            }}
            className="absolute right-1.5 top-1.5 flex h-6 w-6 items-center justify-center rounded-full text-textSecondary transition-colors hover:bg-white/[0.08] hover:text-textPrimary"
          >
            {tucked ? <CaretUp size={12} weight="bold" /> : <CaretDown size={12} weight="bold" />}
          </button>
        </Tooltip>
      )}
      <div className={`flex min-w-0 items-center gap-2.5 ${onTuck ? 'pr-6' : ''}`}>
        <DropArt image={line.image} size={32} radius={8} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[12.5px] font-medium leading-4 text-textPrimary">{line.name}</div>
          <div className="mt-2 flex min-w-0 items-center gap-2.5 text-[11.5px] leading-4">
            <Track line={line} className="h-[3px] min-w-0 flex-1" />
            {line.ready && onClaim ? (
              <ClaimButton line={line} claiming={claiming} onClaim={onClaim} />
            ) : line.connectUrl ? (
              <ConnectButton url={line.connectUrl} className="min-w-[60px] shrink-0" />
            ) : (
              <Status line={line} />
            )}
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
  // Motion Off and Performance mode show the card and the drawer at once.
  // Reduced keeps their fades and drops the slide (MotionScope's
  // reducedMotion); the CSS side is already gated on html[data-motion].
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
  const performanceQuiet = usePerformanceQuiet();
  const instant = motionOff || performanceQuiet;
  // The hover card: a mouse that comes to rest on the lip shows it. Every move
  // restarts the wait, so a pointer crossing the lip on its way to the message
  // box never grows the card (SN-264: it did after 90 ms of passing through).
  // Leaving gives a short grace to reach the card.
  const [peek, setPeek] = useState(false);
  const peekTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hover = (on: boolean) => (e: PointerEvent<HTMLDivElement>) => {
    if (e.pointerType !== 'mouse') return;
    if (peekTimer.current) clearTimeout(peekTimer.current);
    peekTimer.current = setTimeout(() => setPeek(on), on ? PEEK_REST_MS : PEEK_HIDE_MS);
  };
  const rest = (e: PointerEvent<HTMLDivElement>) => {
    if (e.pointerType !== 'mouse' || peek) return;
    if (peekTimer.current) clearTimeout(peekTimer.current);
    peekTimer.current = setTimeout(() => setPeek(true), PEEK_REST_MS);
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
  const tucked = !!view.lip_hidden && !!lead;
  const tuck = (hidden: boolean) => {
    setPeek(false);
    tuckDropsLip(hidden);
  };
  return (
    <div
      ref={hostRef}
      onPointerDownCapture={onPress}
      onPointerEnter={hover(true)}
      onPointerMove={rest}
      onPointerLeave={hover(false)}
      // The lip sits flush on the message box's top edge (over its border), so
      // at rest it takes only its own height from the column. The emote
      // picker opens over this spot: the lip steps out of sight under it but
      // keeps its height, so chat never shifts.
      className={`relative z-[55] -mb-px flex-shrink-0 ${tucked ? '' : 'px-2 pt-1.5'} ${emotePickerOpen ? 'invisible pointer-events-none' : ''}`}
    >
      <AnimatePresence>
        {open && (
          <Suspense key="drawer" fallback={null}>
            <DropsDrawer
              view={view}
              instant={instant}
              maxHeight={maxHeight}
              claimingId={claiming}
              onClaim={(dropId, instanceId) => void claim(dropId, instanceId)}
            />
          </Suspense>
        )}
        {peek && !open && (
          <motion.div
            key="peek"
            // Grows up out of the lip: a short rise and a hair of scale from
            // its bottom edge, quick in and quicker out, transform and opacity
            // only.
            initial={{ opacity: 0, y: 4, scale: 0.985 }}
            animate={{ opacity: 1, y: 0, scale: 1, transition: instant ? INSTANT : PEEK_IN }}
            exit={{ opacity: 0, y: 3, scale: 0.99, transition: instant ? INSTANT : PEEK_OUT }}
            // Above the lip (which comes later and positions itself), so the
            // card covers it whole and grows out of the same place.
            className="absolute bottom-0 left-2 right-2 z-10 origin-bottom"
          >
            <DropsPeekView
              line={line}
              claiming={!!lead && claiming === lead.drop_id}
              onToggle={() => send('toggle')}
              onClaim={lead ? () => void claim(lead.drop_id, lead.instance_id) : undefined}
              tucked={tucked}
              onTuck={lead ? () => tuck(!tucked) : undefined}
            />
          </motion.div>
        )}
      </AnimatePresence>
      {tucked ? (
        <DropsHairline line={line} onShow={() => tuck(false)} onPreload={() => void loadDrawer()} />
      ) : (
        <DropsChatBarView line={line} open={open} onToggle={() => send('toggle')} onPreload={() => void loadDrawer()} />
      )}
    </div>
  );
}

/** Tuck chat's drops lip away into the thin line, or bring it back. Rust
 *  holds the choice and brings the lip back itself when something is worth
 *  showing. */
export function tuckDropsLip(hidden: boolean) {
  void invoke('set_drops_lip_hidden', { hidden }).catch((err) => Logger.warn('[DropsChatBar] tuck failed:', err));
}

/** How long the mouse must stay still on the lip before the card grows. */
const PEEK_REST_MS = 260;
const PEEK_HIDE_MS = 140;
const EASE_OUT = [0.2, 0.8, 0.2, 1] as const;
const PEEK_IN = { duration: 0.14, ease: EASE_OUT };
const PEEK_OUT = { duration: 0.1, ease: EASE_OUT };
const INSTANT = { duration: 0 };

/** Memoized: ChatWidget re-renders on every chat flush and is not compiled,
 *  and this takes only primitives plus one stable callback. */
const DropsChatBar = memo(DropsChatBarInner);
export default DropsChatBar;
