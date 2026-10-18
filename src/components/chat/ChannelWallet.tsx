// The channel wallet: everything the viewer can spend in this channel, in one
// place beside the message box. It grew out of the channel points button and
// keeps everything that button showed (the channel's points icon, the balance
// on the button or on hover, the bonus chest, the "+N"), and now holds bits
// too. Rust owns every number (channel_state: points, the bits face and the
// last gain in each); this only draws them.
//
// `WalletFace` sits in the composer row. `WalletMenu` opens above the whole
// composer, like the insert menu: Points (the rewards menu) and Bits (the
// cheer picker), with the switcher at the bottom when the channel has both.
// `WalletSheet` is the same two tabs inside the phone's bottom sheet.
import { useEffect, useRef, useState, type RefObject } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import ChannelPointsMenu from '../ChannelPointsMenu';
import { ChannelPointsIcon } from '../ChannelPointsIcon';
import { Tooltip } from '../ui/Tooltip';
import { BitsPane } from './BitsPane';
import { MenuSwitcher, type MenuSwitcherTab } from './MenuSwitcher';
import { useFitAbove } from '../../hooks/useFitAbove';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { useAppStore } from '../../stores/AppStore';
import { getBitsState, type BitsState } from '../../services/bitsService';
import { formatShortCount } from '../../utils/streamStats';
import type { ChannelBitsFace, WalletGain } from '../../types';
import { bitsOn } from '../../utils/wallet';

/** The channel's points as the wallet shows them; null = points off here. */
export interface WalletPoints {
  balance: number | null;
  name: string | null;
  iconUrl: string | null;
  loading: boolean;
  lastGain: WalletGain | null;
}

/** `always`: balances on the button. `hover`: on hover only. */
export type WalletBalanceMode = 'always' | 'hover';
/** Which gains get a "+N". */
export type WalletGains = 'all' | 'chest';

const glow = 'transition-all duration-200 group-hover:drop-shadow-[0_0_6px_color-mix(in_srgb,var(--color-accent-neon)_85%,transparent)]';

function PointsGlyph({ iconUrl, name, size, className = '' }: { iconUrl: string | null; name: string | null; size: number; className?: string }) {
  return iconUrl ? (
    <img src={iconUrl} alt={name || 'Channel Points'} style={{ width: size, height: size }} className={`flex-shrink-0 ${className}`} />
  ) : (
    <ChannelPointsIcon size={size} className={`flex-shrink-0 ${className}`} />
  );
}

function Gem({ url, size, className = '' }: { url: string | null; size: number; className?: string }) {
  return url ? <img src={url} alt="" style={{ width: size, height: size }} className={`flex-shrink-0 object-contain ${className}`} /> : null;
}

interface Pop {
  key: string;
  amount: number;
  kind: 'points' | 'bits';
}

/** The seq last seen for one currency: null until its first answer arrived.
 *  The first answer is a baseline, so a gain recorded before this face
 *  existed (an earlier window, a remount) never plays. */
type Seen = { points: number | null; bits: number | null };

/** The "+N" queue: every new gain Rust reports joins it, one plays at a time. */
function useGainPops(
  pointsGain: WalletGain | null,
  pointsReady: boolean,
  bitsGain: WalletGain | null,
  bitsReady: boolean,
  gains: WalletGains,
) {
  const [seen, setSeen] = useState<Seen>({ points: null, bits: null });
  const [queue, setQueue] = useState<Pop[]>([]);
  const [recent, setRecent] = useState<WalletGain | null>(null);
  const next: Seen = {
    points: pointsReady ? pointsGain?.seq ?? 0 : seen.points,
    bits: bitsReady ? bitsGain?.seq ?? 0 : seen.bits,
  };
  if (next.points !== seen.points || next.bits !== seen.bits) {
    setSeen(next);
    const fresh: Pop[] = [];
    if (pointsGain && seen.points !== null && next.points !== null && next.points > seen.points) {
      if (gains === 'all' || pointsGain.chest) fresh.push({ key: `p${next.points}`, amount: pointsGain.amount, kind: 'points' });
      if (pointsGain.reason) setRecent(pointsGain);
    }
    if (bitsGain && gains === 'all' && seen.bits !== null && next.bits !== null && next.bits > seen.bits) {
      fresh.push({ key: `b${next.bits}`, amount: bitsGain.amount, kind: 'bits' });
    }
    if (fresh.length) setQueue((q) => [...q, ...fresh]);
  }
  const head = queue[0] ?? null;
  const headKey = head?.key;
  useEffect(() => {
    if (!headKey) return;
    const t = window.setTimeout(() => setQueue((q) => q.slice(1)), 1800);
    return () => window.clearTimeout(t);
  }, [headKey]);
  // The reason stays on the hover card for five minutes after the gain.
  const recentSeq = recent?.seq;
  useEffect(() => {
    if (!recentSeq) return;
    const t = window.setTimeout(() => setRecent((r) => (r?.seq === recentSeq ? null : r)), 5 * 60_000);
    return () => window.clearTimeout(t);
  }, [recentSeq]);
  return { pop: head, recent };
}

function HoverCard({ points, bits, recent }: { points: WalletPoints | null; bits: ChannelBitsFace | null; recent: WalletGain | null }) {
  return (
    <div className="flex flex-col gap-1">
      {points && (
        <div className="flex items-center gap-1.5">
          <PointsGlyph iconUrl={points.iconUrl} name={points.name} size={14} className={points.iconUrl ? '' : 'text-accent-neon'} />
          {points.loading ? (
            <span className="text-sm text-textSecondary">Loading...</span>
          ) : points.balance !== null ? (
            <span className="text-sm font-bold tabular-nums text-accent-neon">{points.balance.toLocaleString()}</span>
          ) : (
            <span className="text-sm text-textSecondary">--</span>
          )}
          {points.name && points.balance !== null && <span className="text-xs text-textSecondary">{points.name}</span>}
        </div>
      )}
      {points && recent && (
        <span className="pl-5 text-[11.5px] leading-tight text-textSecondary">
          +{recent.amount.toLocaleString()} {recent.reason}
        </span>
      )}
      {bits && (
        <div className="flex items-center gap-1.5">
          <Gem url={bits.gem_url} size={14} />
          {bits.balance !== null ? (
            <span className="text-sm font-bold tabular-nums text-textPrimary">{bits.balance.toLocaleString()}</span>
          ) : (
            <span className="text-sm text-textSecondary">--</span>
          )}
          <span className="text-xs text-textSecondary">bits</span>
        </div>
      )}
    </div>
  );
}

interface FaceProps {
  points: WalletPoints | null;
  bits: ChannelBitsFace | null | undefined;
  mode: WalletBalanceMode;
  gains: WalletGains;
  open: boolean;
  onToggle: () => void;
  /** A bonus chest waiting while auto-collect is off: the face becomes it. */
  chest: { onClaim: () => void; claiming: boolean } | null;
  buttonRef: RefObject<HTMLButtonElement | null>;
}

/** The wallet button in the composer row. */
export function WalletFace({ points, bits: bitsFace, mode, gains, open, onToggle, chest, buttonRef }: FaceProps) {
  const bits = bitsOn(bitsFace);
  const { pop, recent } = useGainPops(
    points?.lastGain ?? null,
    !!points && !points.loading,
    bits?.last_gain ?? null,
    !!bits,
    gains,
  );
  const always = mode === 'always';
  const pointsCount = always && points?.balance != null;
  const bitsCount = always && bits?.balance != null;
  const known = points ? points.balance !== null : bits?.balance != null;
  const label = [
    points && points.balance !== null ? `${points.balance.toLocaleString()} ${points.name || 'channel points'}` : null,
    bits && bits.balance !== null ? `${bits.balance.toLocaleString()} bits` : null,
  ]
    .filter(Boolean)
    .join(', ');

  return (
    <div className="relative flex-shrink-0 self-center flex items-center">
      {chest ? (
        <Tooltip content="Claim bonus points" side="top">
          <button
            onClick={chest.onClaim}
            disabled={chest.claiming}
            aria-label="Claim channel points bonus"
            className="sn-composer-tool chest-attention flex items-center justify-center w-9 h-9 text-accent-neon transition-opacity duration-200 disabled:opacity-50"
          >
            <svg width={18} height={18} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
              <path fillRule="evenodd" clipRule="evenodd" d="M2 22v-9l2.417-4.029A2 2 0 0 1 6.132 8h11.736a2 2 0 0 1 1.715.971L22 13v9H2Zm18-2v-5h-7v2h-2v-2H4v5h16Zm-2.132-10 1.8 3H4.332l1.8-3h11.736Z" />
            </svg>
          </button>
        </Tooltip>
      ) : (
        <Tooltip content={<HoverCard points={points} bits={bits} recent={recent} />} side="top" delay={0} disabled={open}>
          <button
            ref={buttonRef}
            type="button"
            onClick={onToggle}
            aria-label={label ? `Wallet: ${label}` : 'Wallet'}
            aria-expanded={open}
            className={`sn-composer-tool group flex items-center justify-center h-9 gap-1.5 transition-all duration-200 ${
              pointsCount || bitsCount ? 'px-1.5' : 'w-9'
            } ${open || known ? 'text-accent-neon' : 'text-textSecondary hover:text-accent-neon'}`}
          >
            {points ? (
              <span className="relative flex">
                <PointsGlyph iconUrl={points.iconUrl} name={points.name} size={18} className={glow} />
                {/* Bits live here too: a small still gem says so. Hidden while
                    the bits count itself is on the button. */}
                {bits?.gem_url && (
                  <Gem url={bits.gem_url} size={9} className={`absolute -bottom-0.5 -right-1 ${bitsCount ? 'sn-wallet-pip-always' : ''}`} />
                )}
              </span>
            ) : (
              <Gem url={bits?.gem_url ?? null} size={18} className={glow} />
            )}
            {/* Short forms beside the icon; the hover card keeps the exact numbers. */}
            {pointsCount && (
              <span className="sn-composer-points-count text-[12px] font-semibold tabular-nums leading-none whitespace-nowrap">
                {formatShortCount(points!.balance!)}
              </span>
            )}
            {bitsCount && (
              <span className="sn-composer-points-count sn-wallet-bits flex items-center gap-1.5">
                {points && <span className="h-3.5 w-px bg-white/15" aria-hidden="true" />}
                {points && <Gem url={bits!.gem_url} size={12} />}
                <span className="text-[12px] font-semibold tabular-nums leading-none whitespace-nowrap text-textPrimary">
                  {formatShortCount(bits!.balance!)}
                </span>
              </span>
            )}
          </button>
        </Tooltip>
      )}
      {/* "+N" for every gain Rust reports, one after another */}
      <AnimatePresence>
        {pop && (
          <motion.span
            key={pop.key}
            initial={{ opacity: 0, x: '-50%', y: 4, scale: 0.85 }}
            animate={{ opacity: 1, x: '-50%', y: -16, scale: 1 }}
            exit={{ opacity: 0, x: '-50%', y: -24 }}
            transition={{ duration: 0.8, ease: 'easeOut' }}
            className={`absolute left-1/2 top-0 flex items-center gap-0.5 text-[11px] font-semibold pointer-events-none whitespace-nowrap select-none ${
              pop.kind === 'points' ? 'text-accent-neon' : 'text-textPrimary'
            }`}
            aria-live="polite"
          >
            +{pop.amount.toLocaleString()}
            {pop.kind === 'bits' && <Gem url={bits?.gem_url ?? null} size={10} />}
          </motion.span>
        )}
      </AnimatePresence>
    </div>
  );
}

type Tab = 'points' | 'bits';

/** The Bits tab's picker for a channel, read when the tab first shows and
 *  kept with its channel. Rust keeps the balance current through the wallet's
 *  face, so that number wins over the copy from when it opened. */
function useBitsPicker(channelLogin: string, channelId: string, active: boolean, liveBalance: number | null | undefined) {
  const key = `${channelLogin}|${channelId}`;
  const [bitsFor, setBitsFor] = useState<{ key: string; state: BitsState } | null>(null);
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    getBitsState(channelLogin, channelId)
      .then((state) => {
        if (!cancelled) setBitsFor({ key: `${channelLogin}|${channelId}`, state });
      })
      .catch(() => {
        if (!cancelled) setBitsFor(null);
      });
    return () => {
      cancelled = true;
    };
  }, [active, channelLogin, channelId]);
  const state = bitsFor?.key === key ? bitsFor.state : null;
  return state ? { ...state, balance: liveBalance ?? state.balance } : null;
}

const walletTabs = (points: WalletPoints | null, bits: ChannelBitsFace | null): MenuSwitcherTab<Tab>[] => [
  ...(points
    ? [{
        key: 'points' as const,
        label: 'Points',
        icon: <PointsGlyph iconUrl={points.iconUrl} name={points.name} size={15} />,
      }]
    : []),
  ...(bits ? [{ key: 'bits' as const, label: 'Bits', icon: <Gem url={bits.gem_url} size={15} /> }] : []),
];

/** The tab to show: the one picked, unless the channel no longer has it. */
const shownTab = (tab: Tab, points: WalletPoints | null, bits: ChannelBitsFace | null): Tab =>
  tab === 'bits' && !bits ? 'points' : tab === 'points' && !points ? 'bits' : tab;

const bitsLoading = (
  <div className="flex h-40 items-center justify-center" aria-busy="true" aria-label="Loading bits">
    <div className="h-5 w-5 animate-spin rounded-full border-2 border-accent border-t-transparent" />
  </div>
);

interface MenuProps {
  channelLogin: string;
  channelId: string;
  points: WalletPoints | null;
  bits: ChannelBitsFace | null | undefined;
  onClose: () => void;
  /** The face's button: pressing it is its own toggle, not an outside click. */
  ignoreRef: RefObject<HTMLElement | null>;
  onPointsRefresh: () => void | Promise<void>;
  onEmotesChange?: () => void;
  /** Puts a cheermote word in the message box. */
  onInsert: (text: string) => void;
}

const MENU_MAX_PX = 440;

/** The wallet menu, full composer width above the box. Mount it only while open. */
export function WalletMenu({ channelLogin, channelId, points, bits: bitsFace, onClose, ignoreRef, onPointsRefresh, onEmotesChange, onInsert }: MenuProps) {
  const bits = bitsOn(bitsFace);
  const [tab, setTab] = useState<Tab>(points ? 'points' : 'bits');
  const shown = shownTab(tab, points, bits);
  const menuRef = useRef<HTMLDivElement>(null);
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
  useFitAbove(menuRef, true, MENU_MAX_PX);
  const bitsState = useBitsPicker(channelLogin, channelId, shown === 'bits', bits?.balance);

  // A press outside closes it. The Points tab backs out of its own views on
  // Escape; the Bits tab has none, so Escape closes.
  const markInside = useOutsidePress(true, [menuRef, ignoreRef], onClose);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && shown === 'bits') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose, shown]);

  const tabs = walletTabs(points, bits);

  return (
    <motion.div
      ref={menuRef}
      onPointerDownCapture={markInside}
      initial={{ opacity: 0, y: 6, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 6, scale: 0.98 }}
      transition={motionOff ? { duration: 0 } : { duration: 0.16, ease: 'easeOut' }}
      // glass-flyout: this can open over live video, and its base stays
      // readable at every Glassiness.
      className="glass-flyout absolute bottom-full left-0 right-0 mb-2 flex flex-col overflow-hidden z-50 origin-bottom-left !p-0"
      style={{ maxHeight: MENU_MAX_PX }}
    >
      <div className="relative flex min-h-0 flex-1 flex-col">
        {shown === 'points' && points ? (
          <ChannelPointsMenu
            variant="pane"
            channelLogin={channelLogin}
            channelId={channelId}
            currentBalance={points.balance}
            customPointsName={points.name}
            customPointsIconUrl={points.iconUrl}
            onClose={onClose}
            onBalanceUpdate={onPointsRefresh}
            onEmotesChange={onEmotesChange}
          />
        ) : bitsState ? (
          <BitsPane
            state={bitsState}
            onInsert={(text) => {
              onInsert(text);
              onClose();
            }}
          />
        ) : (
          bitsLoading
        )}
      </div>
      {tabs.length > 1 && <MenuSwitcher label="Wallet" tabs={tabs} value={shown} onChange={setTab} />}
    </motion.div>
  );
}

interface SheetProps {
  channelLogin: string;
  channelId: string;
  points: WalletPoints | null;
  bits: ChannelBitsFace | null | undefined;
  onClose: () => void;
  onPointsRefresh: () => void | Promise<void>;
  onEmotesChange?: () => void;
  onInsert: (text: string) => void;
}

/** The phone's wallet: the same two tabs as the desktop menu, inside the
 *  host's bottom sheet, the switcher along the bottom. */
export function WalletSheet({ channelLogin, channelId, points, bits: bitsFace, onClose, onPointsRefresh, onEmotesChange, onInsert }: SheetProps) {
  const bits = bitsOn(bitsFace);
  const [tab, setTab] = useState<Tab>(points ? 'points' : 'bits');
  const shown = shownTab(tab, points, bits);
  const bitsState = useBitsPicker(channelLogin, channelId, shown === 'bits', bits?.balance);
  const tabs = walletTabs(points, bits);
  return (
    <div className="flex flex-col">
      {shown === 'points' && points ? (
        <ChannelPointsMenu
          variant="sheet"
          channelLogin={channelLogin}
          channelId={channelId}
          currentBalance={points.balance}
          customPointsName={points.name}
          customPointsIconUrl={points.iconUrl}
          onClose={onClose}
          onBalanceUpdate={onPointsRefresh}
          onEmotesChange={onEmotesChange}
        />
      ) : bitsState ? (
        <BitsPane
          state={bitsState}
          onInsert={(text) => {
            onInsert(text);
            onClose();
          }}
        />
      ) : (
        bitsLoading
      )}
      {tabs.length > 1 && (
        <div className="sticky bottom-0 mt-2">
          <MenuSwitcher label="Wallet" tabs={tabs} value={shown} onChange={setTab} />
        </div>
      )}
    </div>
  );
}
