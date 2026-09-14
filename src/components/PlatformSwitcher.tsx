// Which platform the app is in, as a quiet readout in the title bar.
//
// The title bar is the only chrome mounted in every view (Home, a category
// drill-down, the player, theater, Compact View), so the answer lives here:
// the platform's mark, how many followed channels are live on it, and a
// chevron, sitting beside the logo. No name in any state: these three logos
// need no caption, and the unified view wears all three of them stacked, which
// says "everything" better than the word did. Earlier versions painted the
// platform's colour along the bar's bottom border, and however dim it was cut it
// still read as decoration; the mark's own colour is the only pigment this
// needs, and text answers the question without decorating anything.
//
// The anchor is a glass PILL (`.titlebar-pill`) with a double chevron. It went
// through a hairline-underline phase — form-field language, on the reasoning
// that its job is to disclose a list rather than to act. The pill puts it back
// in the title bar's own material instead: the bar already carries icon buttons
// on the same radius, so a lone underlined label was the odd one out. The fill
// is a low tint over the bar rather than an opaque plate, and the edge is inset
// light rather than a drawn ring, so it reads as glass and not as a chip.
//
// Hover, focus, or a click opens the choices in a small glass flyout hung
// from the anchor, on the Dynamic Island's spring, so the two share physics
// without sharing a silhouette.
//
// Renders nothing until a second platform's watch support ships, so a
// Twitch-only build is untouched.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { ChevronsUpDown } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import { useFollowsStore } from '../stores/followsStore';
import { usePlatformAccountStore } from '../stores/platformAccountStore';
import { ProviderMark } from './ProviderLogo';
import { streamProvider } from '../utils/streamProvider';
import { WATCHABLE_PROVIDERS, providerLabel, type ProviderId } from '../types/providers';

type Platform = ProviderId | 'all';

const labelOf = (p: Platform): string => (p === 'all' ? 'All platforms' : providerLabel(p));

/** The unified view has no logo of its own, so it borrows everyone's: the
 *  watchable marks in a tight overlapping stack. A generic grid glyph only ever
 *  said "an icon goes here", and it needed the words "All platforms" beside it
 *  to mean anything; three real logos in a row say "all of these" on their own,
 *  which is the actual answer to the question the control is asked.
 *
 *  Laid out as a deck: every mark occupies an identical square footprint and
 *  the squares step diagonally, which is what keeps the cascade even. The three
 *  glyphs are different shapes inside their identical canvases (a tall Twitch
 *  glitch, a blocky Kick K, a wide YouTube plate), so stepping the ink itself
 *  produced a visibly irregular stack. The footprint is pure geometry though:
 *  nothing is painted on it. No tile fill, no ring, no shadow. Both were tried
 *  and both read as logos sitting on chips rather than as a stack of marks. */
function AllMark({ size }: { size: number }) {
  const card = Math.round(size * 0.88);
  const glyph = Math.round(card * 0.72);
  const dx = Math.round(card * 0.44);
  const dy = Math.round(card * 0.3);
  const n = WATCHABLE_PROVIDERS.length;
  const spread = card + dx * (n - 1);
  return (
    // The deck is wider than a single mark, so it occupies the SAME slot the
    // single marks do and centres itself in it, overhanging evenly on both
    // sides. Letting it take its natural width instead left every other row's
    // logo sitting to the left of this one, which is the misalignment you see
    // as "the stack is pushed right".
    <span
      className="relative block flex-shrink-0"
      style={{ width: size, height: card + dy * (n - 1) }}
    >
      {WATCHABLE_PROVIDERS.map((p, i) => (
        <span
          key={p}
          className="absolute flex items-center justify-center"
          style={{
            left: dx * i - (spread - size) / 2,
            top: dy * i,
            width: card,
            height: card,
            // LATER marks sit on top, so the deck leans forward as it goes down
            // and to the right: the top-left card is the one furthest back and
            // the bottom-right is the one nearest you. Stacking it the other way
            // puts the middle card over both of its neighbours, which reads as
            // a pile rather than as a deck.
            zIndex: i + 1,
          }}
        >
          <ProviderMark provider={p} size={glyph} />
        </span>
      ))}
    </span>
  );
}

/** A platform's mark at a fixed size. Deliberately not <ProviderLogo>, which
 *  wraps itself in a Tooltip: inside a control that already spells the name
 *  out, that tooltip is noise. */
function Mark({ platform, size = 13 }: { platform: Platform; size?: number }) {
  if (platform === 'all') return <AllMark size={size} />;
  return (
    <span
      className="flex flex-shrink-0 items-center justify-center leading-none"
      style={{ width: size, height: size }}
    >
      <ProviderMark provider={platform} size={size} />
    </span>
  );
}

export default function PlatformSwitcher() {
  const activePlatform = useAppStore((s) => s.activePlatform);
  const setActivePlatform = useAppStore((s) => s.setActivePlatform);
  const currentStream = useAppStore((s) => s.currentStream);
  const isHomeActive = useAppStore((s) => s.isHomeActive);
  const followedStreams = useAppStore((s) => s.followedStreams);
  const isAuthenticated = useAppStore((s) => s.isAuthenticated);
  const openSettings = useAppStore((s) => s.openSettings);
  const liveByKey = useFollowsStore((s) => s.liveByKey);
  const kickConnected = usePlatformAccountStore((s) => s.kick.connected);
  const youtubeConnected = usePlatformAccountStore((s) => s.youtube.connected);

  const [open, setOpen] = useState(false);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const anchorRef = useRef<HTMLButtonElement | null>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const [menuPos, setMenuPos] = useState<{ top: number; left: number } | null>(null);

  // Portalled to <body> so the flyout escapes the title bar's stacking context
  // (relative z-50): rendered inline, the sidebar overlay (also z-50, later in
  // the DOM) paints over it. Fixed viewport coordinates from the anchor rect,
  // same recipe as the drops hover preview in TitleBar.
  useLayoutEffect(() => {
    if (!open) return;
    const reposition = () => {
      const el = anchorRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      setMenuPos({ top: Math.round(r.bottom + 8), left: Math.round(r.left) });
    };
    reposition();
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);
    return () => {
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
    };
  }, [open]);

  // How many of your follows are live per platform. Twitch's list is Helix's
  // (already live-only); every other platform's comes from the who's-live
  // poller. Both are state we already hold, so the counts cost no request.
  const providerLive = useMemo(() => {
    const by: Partial<Record<ProviderId, number>> = {};
    for (const row of Object.values(liveByKey)) {
      if (row.is_live) by[row.provider] = (by[row.provider] ?? 0) + 1;
    }
    return by;
  }, [liveByKey]);

  // Whether there is an account behind this platform. Twitch's is the app's own
  // sign-in; Kick and YouTube each have one connect action in Settings, however
  // many credentials it actually takes (see platformAccountStore). Written out
  // per platform rather than derived, because "does this have an account at
  // all" is a per-platform fact: the unified view has none to connect, and a
  // platform whose watch support ships without one is connected by definition.
  const isConnected = (p: Platform): boolean => {
    if (p === 'twitch') return isAuthenticated;
    if (p === 'kick') return kickConnected;
    if (p === 'youtube') return youtubeConnected;
    return true;
  };

  const countOf = (p: Platform): number => {
    if (p === 'twitch') return followedStreams.length;
    if (p !== 'all') return providerLive[p] ?? 0;
    return WATCHABLE_PROVIDERS.reduce(
      (n, id) => n + (id === 'twitch' ? followedStreams.length : (providerLive[id] ?? 0)),
      0,
    );
  };

  // "Which platform am I using" is two questions, and answering the wrong one
  // would make this lie half the time. `activePlatform` is the BROWSE scope and
  // can be `all`; the stream playing has its own platform. The player in focus
  // means the stream's platform, browsing means the scope. `isHomeActive` is
  // exactly that toggle, so a Kick stream playing behind the Home tab does not
  // relabel a Twitch-scoped browse.
  const watching = currentStream ? streamProvider(currentStream) : null;
  const focused: Platform = !isHomeActive && watching ? watching : activePlatform;

  const cancelClose = () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
    closeTimer.current = null;
  };
  // A small grace period: the pointer has to cross a few pixels of title bar to
  // get from the anchor down into the flyout, and closing on that gap would
  // make the control feel like it is running away.
  const scheduleClose = () => {
    cancelClose();
    closeTimer.current = setTimeout(() => setOpen(false), 180);
  };
  useEffect(() => cancelClose, []);

  if (WATCHABLE_PROVIDERS.length < 2) return null;

  const options: Platform[] = ['all', ...WATCHABLE_PROVIDERS];

  const select = (p: Platform) => {
    cancelClose();
    setOpen(false);
    setActivePlatform(p);
  };

  // The connect flow is never run from here: Settings > Profile > Accounts is
  // the one place all three platforms are managed, and a second entry point into
  // the same OAuth dance is how those two drift apart.
  const openConnect = () => {
    cancelClose();
    setOpen(false);
    openSettings('Profile', 'settings-section-accounts');
  };

  const moveFocus = (dir: 1 | -1) => {
    const items = itemRefs.current.filter((el): el is HTMLButtonElement => el !== null);
    if (items.length === 0) return;
    const idx = items.indexOf(document.activeElement as HTMLButtonElement);
    const next = idx < 0 ? (dir === 1 ? 0 : items.length - 1) : (idx + dir + items.length) % items.length;
    items[next].focus();
  };

  return (
    <div
      className="relative"
      onMouseEnter={() => {
        cancelClose();
        setOpen(true);
      }}
      onMouseLeave={scheduleClose}
      onFocus={() => {
        cancelClose();
        setOpen(true);
      }}
      onBlur={(e) => {
        // The flyout lives in a body portal, so DOM containment against the
        // wrapper misses it; check both trees by ref.
        const t = e.relatedTarget as Node | null;
        if (!(anchorRef.current?.contains(t) || menuRef.current?.contains(t))) setOpen(false);
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && open) {
          setOpen(false);
          anchorRef.current?.focus();
          e.stopPropagation();
          return;
        }
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          e.preventDefault();
          if (!open) {
            setOpen(true);
            // The rows do not exist until the flyout commits, so focusing the
            // first one has to wait a frame.
            requestAnimationFrame(() => {
              itemRefs.current.find((el): el is HTMLButtonElement => el !== null)?.focus();
            });
            return;
          }
          moveFocus(e.key === 'ArrowDown' ? 1 : -1);
        }
      }}
    >
      {/* The anchor wears `titlebar-select`: a hairline under the content and
          nothing else. See that class for the treatments this replaced and why
          each one failed. */}
      <button
        ref={anchorRef}
        type="button"
        aria-label={`Platform: ${labelOf(focused)}, ${
          isConnected(focused) ? `${countOf(focused)} followed live` : 'not connected'
        }`}
        aria-expanded={open}
        aria-haspopup="menu"
        onClick={() => setOpen((v) => !v)}
        // A pill again, so the padding comes back: the fill needs room around
        // the content or it reads as a plate clamped to the text. Taller too —
        // 22px was sized for a hairline with nothing above or below it.
        className={`titlebar-pill flex h-[26px] items-center gap-[7px] pl-[9px] pr-[7px] outline-none ${
          open ? 'is-open' : ''
        }`}
      >
        {/* A size larger than the flyout's marks: with the name gone this is
            the only thing identifying the platform, so it carries the weight
            the words used to. */}
        <Mark platform={focused} size={13} />
        {/* Baseline-aligned, not centre-aligned. Two different type sizes on
            one line centre by their em boxes, which puts their baselines a
            fraction apart and reads as the smaller one sitting low. */}
        <span className="flex items-baseline gap-[6px]">
          {/* No name in any state now. Every platform says its own name with
              its logo, and the unified view says it with all three of them, so
              the words were width spent on something the marks already carry.
              The flyout still names each one in full the moment you hover. */}
          {/* Same rule as the rows: with no account behind the platform, "0
              live" would be a claim about your follows when the real answer is
              that there is nothing signed in to ask. */}
          <span
            className={`hidden text-[10.5px] leading-none text-textMuted min-[700px]:inline ${
              isConnected(focused) ? 'tabular-nums' : ''
            }`}
          >
            {isConnected(focused) ? `${countOf(focused)} live` : 'not connected'}
          </span>
        </span>
        {/* Double chevron: select language rather than disclosure language. A
            single one points DOWN and so promises the list opens downward; this
            one says "there are other values here" without claiming a direction,
            which is the honest glyph for a control whose flyout can flip above
            the anchor. It also means nothing has to rotate on open — the shape
            is already symmetric, so the state is carried by brightness alone. */}
        <ChevronsUpDown
          size={11}
          strokeWidth={2.5}
          className="flex-shrink-0 text-textMuted"
          style={{
            opacity: open ? 0.85 : 0.5,
            transition: 'opacity 150ms ease',
          }}
        />
      </button>

      {createPortal(
      <AnimatePresence>
        {open && menuPos && (
          <motion.div
            ref={menuRef}
            role="menu"
            aria-label="Platform"
            // glass-flyout, not glass-panel: this hangs over live video, and
            // the panel recipe thins out to a translucent sheet at low
            // Glassiness. The flyout recipe keeps a real frost at every slider
            // position (see globals.css for the why).
            className="glass-flyout w-[196px] overflow-hidden"
            style={{ position: 'fixed', top: menuPos.top, left: menuPos.left, zIndex: 9999, transformOrigin: 'top left' }}
            initial={{ opacity: 0, scale: 0.96, y: -5 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.97, y: -4, transition: { duration: 0.12, ease: 'easeIn' } }}
            // The Dynamic Island's spring, on purpose: same physics, so the two
            // read as one family. Opacity is excluded from the spring because a
            // spring's settle time makes a fade look like a slow dissolve.
            transition={{
              type: 'spring',
              stiffness: 360,
              damping: 32,
              mass: 0.9,
              opacity: { duration: 0.15, ease: 'easeOut' },
            }}
          >
            <div className="p-1">
              {options.map((p, i) => {
                const connected = isConnected(p);
                const on = p === activePlatform;
                return (
                  // Two targets, one row. Kick and YouTube directories are
                  // public, so picking an unconnected platform still browses it
                  // anonymously; only the word "Connect" goes to Settings. They
                  // are siblings rather than nested, because a button inside a
                  // button is invalid and the nested one stops being reachable.
                  <div key={p} className="relative">
                  <button
                    ref={(el) => {
                      itemRefs.current[i] = el;
                    }}
                    type="button"
                    role="menuitemradio"
                    aria-checked={on}
                    onClick={() => select(p)}
                    // Selection is a LIT SURFACE (see .platform-row). Still no
                    // brand wash and no colour bar per row: the marks are
                    // already three saturated colours, and tinting the rows on
                    // top of them is what made this read as a toy. The flat
                    // white-alpha fill this replaces was the same material as a
                    // disabled control, which is why the chosen row read as
                    // "greyed" rather than "chosen".
                    className={`platform-row relative flex w-full items-center gap-2.5 py-[7px] pl-2.5 text-left outline-none ${
                      // Room for the Connect button sitting over the right of
                      // this row, so a long platform name cannot run under it.
                      connected ? 'pr-2.5' : 'pr-[62px]'
                    } ${on ? 'is-selected' : ''}`}
                  >

                    {/* Every mark at full strength. Fading the unselected ones
                        back was cheap emphasis paid for with the brands: a
                        Kick green at 55% over a dark panel is not Kick green,
                        it is a muddy olive, and the same goes for every logo
                        here. The row fill and the text colour carry the
                        selected state on their own. */}
                    <Mark platform={p} size={12} />
                    {/* A real line-height, not `leading-none`. `truncate` sets
                        overflow:hidden, and a line box exactly as tall as the
                        font has no room for the descenders in "All platforms"
                        and "YouTube", so the tails of the p and the y were
                        being shaved off by the span's own clipping. */}
                    <span
                      className={`truncate text-[11.5px] leading-[15px] ${on ? 'text-textPrimary' : 'text-textSecondary'}`}
                    >
                      {labelOf(p)}
                    </span>
                    {/* Only a connected platform can say how many of your
                        follows are live. Without an account, "0 live" would be
                        a claim about your follows when the truth is that there
                        is nothing signed in to ask. */}
                    {connected && (
                      <span
                        className={`ml-auto text-[10.5px] leading-none tabular-nums ${
                          on ? 'text-textSecondary' : 'text-textMuted'
                        }`}
                      >
                        {countOf(p)} live
                      </span>
                    )}
                  </button>
                  {!connected && (
                    <button
                      type="button"
                      onClick={() => openConnect()}
                      className="absolute right-2 top-1/2 -translate-y-1/2 rounded-md px-1.5 py-1 text-[10.5px] leading-none text-textMuted outline-none transition-colors duration-150 hover:bg-white/[0.07] hover:text-textPrimary focus-visible:bg-white/[0.07]"
                    >
                      Connect
                    </button>
                  )}
                  </div>
                );
              })}
            </div>
            {/* A provider stream playing outside the browse scope is stated
                outright, as a footer, so switching the scope never looks like
                it moved the player. */}
            {watching && watching !== activePlatform && (
              <div
                className="flex items-center gap-2 px-3 pb-2 pt-[7px]"
                style={{
                  borderTop: '1px solid color-mix(in srgb, var(--color-border-subtle) 55%, transparent)',
                }}
              >
                <Mark platform={watching} size={10} />
                {/* A real line-height, not `leading-none`. This is the last
                    element in a panel that clips its overflow to round its
                    corners, and a line box exactly as tall as the font has no
                    room for the descenders in "Watching" and "YouTube", so the
                    tails of the g and the y were being shaved off. */}
                <span className="truncate text-[10.5px] leading-[14px] text-textMuted">
                  Watching on {providerLabel(watching)}
                </span>
              </div>
            )}
          </motion.div>
        )}
      </AnimatePresence>,
      document.body)}
    </div>
  );
}
