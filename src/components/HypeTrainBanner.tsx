import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { AnimatePresence, motion } from 'framer-motion';
import { Gift } from 'phosphor-react';
import type { HypeTrainData } from '../types';
import ConfettiBurst from './ConfettiBurst';
import { Logger } from '../utils/logger';
import { formatShortCount } from '../utils/streamStats';
import { fitHeaderDensity } from './chat/fitHeaderDensity';
import { GLAZE_BANNER } from './ui/glazeButtons';
import { useAppStore } from '../stores/AppStore';
import { usePerformanceQuiet } from '../utils/lowPower';
import { parseBadges } from '../services/twitchBadges';
import { HYPE_LOOK_FILL, hypeTrainLook } from '../utils/hypeTrainLook';

// The decorated Twitch Hype Train bar, extracted from ChatWidget so every
// surface (the main player chat, MultiChat split/tab panes, and the blended
// feed) renders the exact same thing. Owns its own countdown, level-up
// celebration, and confetti. The bar renders inline where it's placed; the
// level-up confetti is portalled into `confettiTarget` so it can rain over the
// whole chat/feed instead of just the bar's height.

const HYPE_MESSAGES = [
  // Classic hype
  'HYYYYPE! 🚂',
  'CHOO CHOO! ALL ABOARD! 🚂💨',
  'ALL ABOARD THE HYPE TRAIN LET\'S GOOO 🎉',
  'WE EATING GOOD TONIGHT 🍽️🔥',
  'POGGERS IN CHAT 🐸',
  'TRAIN HAS LEFT THE STATION AND IT\'S ON FIRE 🚂🔥',
  'LET\'S GOOOOOOOOOOOOO 🔥',
  'CHAT POPPIN OFF RN 📈',
  'THIS IS THE ENERGY WE CAME FOR 🙌',
  'TURN IT UP TO ELEVEN 🔊✨',
  'CHAT IS GLOWING RIGHT NOW 🌟',
  // Pure good vibes
  'THE VIBES ARE IMMACULATE 🤩✨',
  'BIGGEST W OF THE NIGHT 🏆',
  'EVERYBODY IS SO BACK 🙌🔥',
  'THIS IS PEAK PERFORMANCE 📈💯',
  'CHAT YOU ARE INCREDIBLE 💖',
  'MAXIMUM HYPE ACHIEVED 🚀',
  'WE ARE SO LOCKED IN 🔒🔥',
  'GREATEST TIMELINE CONFIRMED ✨',
  'CHAT IS UNSTOPPABLE TONIGHT 💪',
  'THIS IS WHAT DREAMS LOOK LIKE 🌈',
  // Train-themed
  'FULL STEAM AHEAD! 🚂💨',
  'NEXT STOP: THE STRATOSPHERE 🚂🌌',
  'THIS TRAIN HAS NO BRAKES 🚂⚡',
  'ENGINE\'S REDLINING AND WE LOVE IT 🚂🔥',
  'BUCKLE UP, WE\'RE GOING UP 🚂📈',
  'CONDUCTOR SAID ONE MORE LEVEL 🚂🎩',
  'RIDE THIS TRAIN TO THE MOON 🚂🌙',
  // StreamNook-branded hype
  'ALL ABOARD THE STREAMNOOK HYPE TRAIN! 🎉',
  'STREAMNOOK FAM LET\'S GOOO 🔥',
  'POGGERS IN THE NOOK 🐸🏠',
  'TRAIN HAS LEFT THE STATION AND STREAMNOOK IS DRIVING 🚂🌪️',
  'CHAT POPPIN OFF IN STREAMNOOK RN 📈',
  'STREAMNOOK ENERGY IS UNMATCHED 🙌',
  'COZIEST HYPE IN THE NOOK TONIGHT 🛋️🚂',
  'THE NOOK IS GLOWING ✨🏠',
  'NOBODY DOES IT LIKE THE NOOK 💯',
  'STREAMNOOK FAM ON TOP AS USUAL 🏆',
  'THIS NOOK MOMENT IS LEGENDARY 🌟🚂',
  'WELCOME TO THE BEST SEAT IN THE NOOK 🛋️🔥',
];

interface HypeTrainBannerProps {
  train: HypeTrainData;
  /** The element the level-up confetti renders into (so it covers the whole
   *  chat/feed, not just the bar). Pass the scroll/chat container element. When
   *  omitted, the bar + flash still play but no full-area confetti rains. */
  confettiTarget?: HTMLElement | null;
  /** When set, the owner throws the level-up confetti itself (so what lands
   *  on its composer outlives the bar), and `confettiTarget` is not used. */
  onLevelUp?: (golden: boolean) => void;
  /** Fired when the countdown reaches 0, so the owner can clear its own state.
   *  The main player chat clears the global hype train here; pane/blended
   *  pollers manage their own removal, so they can leave this unset. */
  onExpire?: () => void;
  /** The card floats over moving chat rows (the chat header), so it wears
   *  the full glaze with its blur. Unset, it sits on an opaque strip and the
   *  flat glaze (no blur to buy) is right. */
  floating?: boolean;
  /** The phone's chat: the same two rows, packed tighter (a 16 px top row, a
   *  track row no taller than the track, no margin above), so the card is
   *  42 px instead of 52 and starts 8 px higher. Every number keeps its slot. */
  compact?: boolean;
}

export default function HypeTrainBanner({ train, confettiTarget, onLevelUp, onExpire, floating = false, compact = false }: HypeTrainBannerProps) {
  // The top row's height and line, in one place, so the chips, the level and
  // the words always share it.
  const rowH = compact ? 'h-4' : 'h-[18px]';
  const lead = compact ? 'leading-4' : 'leading-[18px]';
  const [timeRemaining, setTimeRemaining] = useState('');
  // The share of the level's window still left (the timer's outline), and
  // how warm the timer runs (heatFor) as the window closes.
  const [clockFrac, setClockFrac] = useState(1);
  const [clockHeat, setClockHeat] = useState(0);
  const expiresAtRef = useRef<string | null>(null);
  const [isLevelUpCelebration, setIsLevelUpCelebration] = useState(false);
  const previousLevelRef = useRef<number>(0);
  const previousProgressRef = useRef<number>(0);
  const [displayedLevel, setDisplayedLevel] = useState<number>(0);
  const [celebrationMessage, setCelebrationMessage] = useState<string>('');
  // Bumped on each level-up so the confetti burst remounts and replays.
  const [celebrationId, setCelebrationId] = useState(0);
  const onExpireRef = useRef(onExpire);
  onExpireRef.current = onExpire;
  const onLevelUpRef = useRef(onLevelUp);
  onLevelUpRef.current = onLevelUp;
  // The level side steps down in place while it overflows (CSS
  // `.hype-train-level[data-density]` says what each step drops).
  const [levelEl, setLevelEl] = useState<HTMLDivElement | null>(null);
  useEffect(() => (levelEl ? fitHeaderDensity(levelEl, 2, 0) : undefined), [levelEl]);
  // The level the celebration announces. A bar reset can fire it a beat
  // before Twitch's level field moves, so it is worked out at the trigger.
  const [celebrationLevel, setCelebrationLevel] = useState(0);
  // Motion Off and Performance mode: the level number swaps instead of
  // rolling, and the words stand still (reduced motion too).
  const motionMode = useAppStore((s) => s.settings?.motion_mode);
  const perfQuiet = usePerformanceQuiet();
  const instant = motionMode === 'off' || perfQuiet;
  const stillWords = motionMode === 'off' || motionMode === 'reduced' || prefersReducedMotion();

  // Smooth countdown, updated every second locally. On hitting 0 it fires
  // onExpire (the owner clears the train) rather than waiting for the next poll.
  // The ring measures what is left against the window the level had when its
  // timer last moved (a contribution or a level-up resets it), since Twitch
  // sends only the end time.
  useEffect(() => {
    if (!train.expires_at) {
      setTimeRemaining('');
      setClockFrac(1);
      setClockHeat(0);
      expiresAtRef.current = null;
      return;
    }
    expiresAtRef.current = train.expires_at;
    const windowMs = Math.max(1000, new Date(train.expires_at).getTime() - Date.now());
    const tick = () => {
      if (!expiresAtRef.current) return;
      const diffMs = Math.max(0, new Date(expiresAtRef.current).getTime() - Date.now());
      if (diffMs === 0) {
        onExpireRef.current?.();
        return;
      }
      const minutes = Math.floor(diffMs / 60000);
      const seconds = Math.floor((diffMs % 60000) / 1000);
      setTimeRemaining(`${minutes}:${seconds.toString().padStart(2, '0')}`);
      setClockFrac(Math.min(1, diffMs / windowMs));
      setClockHeat(heatFor(diffMs / 1000));
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [train.expires_at]);

  // Detect a level-up and START the celebration. Fire on EITHER the level field
  // incrementing OR the progress bar resetting (a sharp progress drop). Twitch's
  // status resets progress a beat BEFORE the level field updates, so keying only
  // off `level` made the confetti lag the visible bar reset by a beat. Progress
  // only ever climbs within a level, so a sharp drop is an unambiguous level-up.
  useEffect(() => {
    const prevLevel = previousLevelRef.current;
    const prevProgress = previousProgressRef.current;
    // The bar also empties when the train ENDS (it resets as it concludes), which
    // must NOT fire a celebration that then gets cut off when the train clears.
    // So the progress-reset path only counts while the train is genuinely still
    // running (timer comfortably in the future). The level-increment path is
    // inherently safe: an ending train never increments its level.
    const stillRunning =
      !train.expires_at || new Date(train.expires_at).getTime() - Date.now() > 3000;
    const leveledUp = train.level > prevLevel && prevLevel > 0;
    const progressReset =
      stillRunning && prevProgress > 0 && train.progress < prevProgress * 0.5;
    previousLevelRef.current = train.level;
    previousProgressRef.current = train.progress;
    if (!isLevelUpCelebration && (leveledUp || progressReset)) {
      Logger.debug(`[HypeTrain] LEVEL UP (level ${prevLevel} -> ${train.level}, barReset=${progressReset})`);
      setCelebrationMessage(HYPE_MESSAGES[Math.floor(Math.random() * HYPE_MESSAGES.length)]);
      setCelebrationLevel(leveledUp ? train.level : Math.max(train.level, prevLevel) + 1);
      setIsLevelUpCelebration(true);
      setCelebrationId((id) => id + 1);
      onLevelUpRef.current?.(!!train.is_golden_kappa);
    } else if (!isLevelUpCelebration) {
      setDisplayedLevel(train.level);
    }
    // isLevelUpCelebration is read but intentionally omitted from deps: including
    // it would re-run this on celebration start/end and risk re-triggering.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [train.level, train.progress]);

  // The celebration ends when the moving words have left the strip
  // (`endCelebration` from LevelUpWords), so the row never sits empty while
  // it waits. Still words stand for STILL_MS. The timer is the backstop either
  // way, keyed on celebrationId so progress ticks during the celebration don't
  // reset it (only a fresh level-up, which bumps celebrationId, restarts it).
  const endCelebration = () => {
    setIsLevelUpCelebration(false);
    setDisplayedLevel(previousLevelRef.current);
  };
  useEffect(() => {
    if (celebrationId === 0) return;
    const t = setTimeout(() => {
      setIsLevelUpCelebration(false);
      setDisplayedLevel(previousLevelRef.current);
    }, stillWords ? STILL_MS : WORDS_MAX_MS + 1500);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [celebrationId]);

  // Guard against NaN when goal is 0 or undefined.
  const percentage = train.goal > 0
    ? Math.min(Math.round((train.progress / train.goal) * 100), 100)
    : 0;
  const remaining = Math.max(0, train.goal - train.progress);
  const bitsNeeded = remaining; // 1 bit = 1 point
  const subsNeeded = Math.ceil(remaining / 500); // 1 Tier1 sub = 500 points
  const look = hypeTrainLook(train);
  const isGolden = look === 'golden';
  const bitsGem = useBitsGem(train.broadcaster_user_login, bitsNeeded);
  // The track fills to the brim while a level-up is celebrated.
  const shown = isLevelUpCelebration ? 100 : percentage;

  return (
    <>
      {/* Level-up confetti. Bursts up from the bar and falls the full height of
          the host container. Portalled so it covers the whole chat/feed. */}
      {isLevelUpCelebration && confettiTarget && !onLevelUp &&
        createPortal(<ConfettiBurst key={celebrationId} golden={!!isGolden} />, confettiTarget)}

      {/* The bar: a glaze card with two rows. The top row says where the
          train stands (level, what is still needed, time left); the bottom is
          an inset track with the percentage at its end. On a level-up the top
          row makes way for the words and the track fills to the brim. */}
      <div
        className={`hype-train-card ${GLAZE_BANNER} ${floating ? '' : 'chrome-glaze--flat'} relative ${compact ? '' : 'mt-2'} overflow-hidden pointer-events-none`}
        style={CARD_STYLE}
        data-look={look}
      >
        <div className={`relative px-2.5 ${compact ? 'pt-[5px] pb-2' : 'pt-[7px] pb-2'}`}>
          <div className={`relative ${rowH}`}>
            <div
              className={`absolute inset-0 grid grid-cols-[minmax(0,1fr)_auto] items-center gap-2 transition-opacity duration-200 ${
                isLevelUpCelebration ? 'opacity-0' : 'opacity-100'
              }`}
            >
              {/* Level and what is still needed. Too narrow for all of it,
                  the subs go first, then the bits; the level stays whole. */}
              <div ref={setLevelEl} className="hype-train-level flex min-w-0 items-center gap-2 overflow-hidden whitespace-nowrap">
                <span className={`text-[12.5px] font-semibold ${lead} text-textPrimary`}>
                  Level <LevelNumber level={displayedLevel || train.level} instant={instant} rowH={rowH} />
                </span>
                {remaining > 0 ? (
                  // What the next level still needs, as marks: the bits gem and
                  // the sub star, each with its count, in one quiet chip.
                  <span
                    className="hype-train-left flex items-center gap-1.5"
                    aria-label={`${formatShortCount(bitsNeeded)} bits or ${subsNeeded} ${subsNeeded === 1 ? 'sub' : 'subs'} to go`}
                  >
                    <span className={`hype-train-need flex ${rowH} items-center gap-1.5 rounded-full px-[7px] text-[11px] font-semibold text-textPrimary tabular-nums`} aria-hidden>
                      <span className="flex items-center gap-1">
                        <BitsMark gem={bitsGem} />
                        {formatShortCount(bitsNeeded)}
                      </span>
                      <span className="hype-train-subs flex items-center gap-1.5">
                        <span className="hype-train-need-divider h-[9px] w-px" />
                        <span className="flex items-center gap-1">
                          <SubMark />
                          {subsNeeded}
                        </span>
                      </span>
                    </span>
                    <span className={`text-[11.5px] ${lead} text-textSecondary`} aria-hidden>to go</span>
                  </span>
                ) : (
                  <span className={`hype-train-left text-[11.5px] ${lead} text-textSecondary`}>Level up incoming</span>
                )}
              </div>
              {timeRemaining && (
                // Time left, in the same recess as the need chip: a ring that
                // empties with the level's window, turning to the warning
                // color for the last stretch along with the digits.
                <DrainClock time={timeRemaining} frac={clockFrac} heat={clockHeat} rowH={rowH} />
              )}
            </div>
            {isLevelUpCelebration && (
              <LevelUpWords
                key={celebrationId}
                level={celebrationLevel || train.level}
                message={celebrationMessage}
                still={stillWords}
                lead={lead}
                onDone={endCelebration}
              />
            )}
          </div>
          {/* Compact, the row is the track's own 6 px and the percentage
              centers on it, overhanging the gaps above and below. */}
          <div className={`${compact ? 'h-1.5' : ''} mt-[7px] flex items-center gap-3`}>
            <div className="relative h-1.5 min-w-0 flex-1">
              {/* A soft glow under the fill, revealed with it. Its gradient is
                  still (the blur is drawn once; only the clip moves). */}
              <div
                className="hype-train-meter-glow absolute inset-0 rounded-full"
                // Overhang wide enough for the whole blur to fade out inside it,
                // so the clip never shows as an edge; its rounded end stops
                // under the train's body, never ahead of the train.
                style={{ clipPath: `inset(-14px calc(${100 - shown}% + 8px) -14px -14px round 999px)` }}
                aria-hidden
              />
              <div className="hype-train-meter absolute inset-0 overflow-hidden rounded-full">
                <div
                  className={`hype-train-meter-fill absolute inset-0 ${HYPE_LOOK_FILL[look]}`}
                  style={{ clipPath: `inset(0 ${100 - shown}% 0 0 round 999px)` }}
                >
                  {/* Now and then a light runs along the fill. */}
                  <span className="hype-train-shimmer" aria-hidden />
                </div>
                {isLevelUpCelebration && <span key={celebrationId} className="hype-train-sheen" aria-hidden />}
              </div>
              {/* The train rides the fill's leading edge like a comet: a flash
                  at its nose, a tapering trail and a few sparks behind it.
                  All of it is carried by a full-width rail moved on a
                  transform; the trail is no longer than the fill. */}
              {shown > 0 && (
                <div
                  className="hype-train-tip-rail pointer-events-none absolute inset-0"
                  style={{ transform: `translateX(${shown - 100}%)` }}
                  aria-hidden
                >
                  <span className="hype-train-comet" style={{ maxWidth: `${shown}%` }} />
                  <span className="hype-train-sparks" style={{ maxWidth: `${shown}%` }}>
                    <i />
                    <i />
                    <i />
                    <i />
                  </span>
                  <span className="hype-train-flash" />
                  <RiderTrain />
                </div>
              )}
            </div>
            {/* Sized for "100%", so the track never shifts as it counts. */}
            <span className="w-[34px] shrink-0 text-right text-[11.5px] font-bold leading-none text-textPrimary tabular-nums">
              {shown}%
            </span>
          </div>
        </div>
      </div>
    </>
  );
}

/** The card's corner and frost (the glaze rounds to a pill on its own). */
const CARD_STYLE = { borderRadius: 12, '--glaze-frost': '24%' } as CSSProperties;

const prefersReducedMotion = () =>
  typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

/** How warm the timer runs as a level's window closes, from the seconds
 *  left: 0 (calm) above HEAT_WARM_S, rising to 1 (the warning amber) at
 *  HEAT_HOT_S, then to 2 (red) at HEAT_RED_S. It moves a little every second,
 *  and each step is blended over that second, so the color never jumps. */
const HEAT_WARM_S = 60;
const HEAT_HOT_S = 20;
const HEAT_RED_S = 5;
function heatFor(secondsLeft: number): number {
  if (secondsLeft >= HEAT_WARM_S) return 0;
  if (secondsLeft >= HEAT_HOT_S) return (HEAT_WARM_S - secondsLeft) / (HEAT_WARM_S - HEAT_HOT_S);
  if (secondsLeft >= HEAT_RED_S) return 1 + (HEAT_HOT_S - secondsLeft) / (HEAT_HOT_S - HEAT_RED_S);
  return 2;
}
/** The color at `heat` from a calm `base`: mixed in oklab, so blue to amber
 *  to red passes through clean hues instead of muddy ones. */
function heatColor(heat: number, base: string): string {
  if (heat <= 0) return base;
  if (heat <= 1) return `color-mix(in oklab, var(--color-warning) ${(heat * 100).toFixed(1)}%, ${base})`;
  return `color-mix(in oklab, var(--color-error) ${((heat - 1) * 100).toFixed(1)}%, var(--color-warning))`;
}

/** Time left: the chip's own outline is the countdown, a lit line that
 *  starts full at the top center and drains back to it, clockwise, as time
 *  runs out, warming from the bar's blue through amber to red. The outline is
 *  a path traced around the chip's measured box. */
function DrainClock({ time, frac, heat, rowH }: { time: string; frac: number; heat: number; rowH: string }) {
  const label = `${time} left`;
  const ref = useRef<HTMLSpanElement>(null);
  const [box, setBox] = useState<{ w: number; h: number } | null>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setBox((b) => (b && b.w === el.offsetWidth && b.h === el.offsetHeight ? b : { w: el.offsetWidth, h: el.offsetHeight }));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  let d = '';
  if (box) {
    const i = 0.75;
    const x0 = i;
    const y0 = i;
    const x1 = box.w - i;
    const y1 = box.h - i;
    const r = (y1 - y0) / 2;
    const cx = box.w / 2;
    d = `M${cx} ${y0}H${x1 - r}A${r} ${r} 0 0 1 ${x1 - r} ${y1}H${x0 + r}A${r} ${r} 0 0 1 ${x0 + r} ${y0}Z`;
  }
  return (
    <span
      ref={ref}
      className={`hype-train-need hype-train-clock relative flex ${rowH} shrink-0 items-center rounded-full px-[9px] text-[11px] font-semibold tabular-nums`}
      style={{ color: heatColor(heat, 'var(--color-text-primary)') }}
      aria-label={label}
    >
      {box && (
        <svg
          className="hype-train-clock-line pointer-events-none absolute inset-0 overflow-visible"
          width={box.w}
          height={box.h}
          style={{ color: heatColor(heat, 'var(--hype-clock-base)') }}
          aria-hidden
        >
          <path d={d} fill="none" strokeWidth="1.5" className="hype-train-clock-track" />
          <path
            d={d}
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            pathLength={100}
            strokeDasharray="100"
            strokeDashoffset={100 * (1 - frac)}
            className="hype-train-clock-left"
          />
        </svg>
      )}
      <span className="relative" aria-hidden>
        {time}
      </span>
    </span>
  );
}

interface MarkArt {
  url: string | null;
  /** The ask has answered (with art or without). */
  settled: boolean;
}

/** The subs mark in the need chip: Twitch's gift sub art (the Sub Gifter
 *  badge), the same art chat's gift notices wear (ChatMessage's
 *  SubGiftIcon), from the global badge cache every chat connection fills.
 *  A drawn gift stands in only until that cache has loaded; the banner
 *  re-renders each second with its countdown, so the art arrives on its own. */
function SubMark() {
  const art = parseBadges('sub-gifter/1')[0]?.info?.image_url_2x as string | undefined;
  return art ? (
    <img src={art} alt="" className="h-3 w-3 shrink-0 object-contain" draggable={false} />
  ) : (
    <Gift size={11} weight="fill" className="hype-train-sub-gift shrink-0" />
  );
}

/** Twitch's still Cheer gem for `amount` bits in the channel's cheer set,
 *  the gem the wallet wears (grey, purple, green, blue or red by tier), from
 *  Rust (`bits_gem`, an in-memory lookup). Between amounts the last gem stays
 *  up, so a new count never blinks it. */
function useBitsGem(login: string | undefined, amount: number): MarkArt {
  const key = login ? `${login.toLowerCase()}:${amount}` : '';
  const [held, setHeld] = useState<{ key: string; url: string | null } | null>(null);
  useEffect(() => {
    if (!key || !login || amount <= 0) return;
    let cancelled = false;
    void invoke<string | null>('bits_gem', { channelLogin: login, amount })
      .then((url) => {
        if (!cancelled) setHeld({ key, url: url || null });
      })
      .catch((err) => {
        Logger.warn('[HypeTrain] bits gem:', err);
        if (!cancelled) setHeld({ key, url: null });
      });
    return () => {
      cancelled = true;
    };
  }, [key, login, amount]);
  return held ? { url: held.url, settled: true } : { url: null, settled: false };
}

/** The bits mark in the need chip: Twitch's gem for the amount; a blank while
 *  it is asked for; the drawn standard gem only when the channel's cheer set
 *  is not known. */
function BitsMark({ gem }: { gem: MarkArt }) {
  if (gem.url) {
    return <img src={gem.url} alt="" className="h-3 w-3 shrink-0 object-contain" referrerPolicy="no-referrer" draggable={false} />;
  }
  if (!gem.settled) return <span className="h-3 w-3 shrink-0" />;
  return <BitsGem />;
}

/** A drawn standard purple bits gem, for a channel whose cheer set is not
 *  known yet. */
function BitsGem() {
  return (
    <svg className="h-[11px] w-[11px] shrink-0" viewBox="0 0 16 16" aria-hidden>
      <path d="M8 1 14.5 6.6 8 15 1.5 6.6Z" fill="#9147ff" />
      <path d="M8 1 11.2 6.6H4.8Z" fill="#c9a8ff" />
      <path d="M4.8 6.6H11.2L8 15Z" fill="#7a2ff0" />
    </svg>
  );
}

/** The hype train glyph riding the bar, mirrored so its smokestack leads
 *  the way it travels (left to right). Its wheels sit on the track. */
function RiderTrain() {
  return (
    <svg className="hype-train-rider" viewBox="0 0 15 13" fill="none">
      <path fillRule="evenodd" clipRule="evenodd" d="M4.10001 0.549988H2.40001V4.79999H0.700012V10.75H1.55001C1.55001 11.6889 2.31113 12.45 3.25001 12.45C4.1889 12.45 4.95001 11.6889 4.95001 10.75H5.80001C5.80001 11.6889 6.56113 12.45 7.50001 12.45C8.4389 12.45 9.20001 11.6889 9.20001 10.75H10.05C10.05 11.6889 10.8111 12.45 11.75 12.45C12.6889 12.45 13.45 11.6889 13.45 10.75H14.3V0.549988H6.65001V2.24999H7.50001V4.79999H4.10001V0.549988ZM12.6 9.04999V6.49999H2.40001V9.04999H12.6ZM9.20001 4.79999H12.6V2.24999H9.20001V4.79999Z" fill="currentColor" />
    </svg>
  );
}

/** The level number, rolling up inside its own line when it changes. */
function LevelNumber({ level, instant, rowH }: { level: number; instant: boolean; rowH: string }) {
  return (
    <span className={`relative inline-flex ${rowH} overflow-hidden align-top tabular-nums`}>
      <AnimatePresence initial={false} mode="popLayout">
        <motion.span
          key={level}
          initial={{ y: '100%', opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: '-100%', opacity: 0 }}
          transition={instant ? { duration: 0 } : { duration: 0.34, ease: [0.2, 0.8, 0.2, 1] }}
        >
          {level}
        </motion.span>
      </AnimatePresence>
    </span>
  );
}

/** The words cross at a steady reading pace, whatever their length or the
 *  strip's width, within these bounds; the celebration ends as they leave. */
const WORDS_PX_PER_S = 95;
const WORDS_MIN_MS = 4500;
const WORDS_MAX_MS = 9500;
/** How long still words (motion off or reduced) stand. */
const STILL_MS = 5000;

/** The level-up words. They glide in from the right and leave on the left
 *  once, behind soft edge fades, and `onDone` fires as they leave, so they
 *  are never cut off. With motion off or reduced they stand still in the
 *  middle, ending in an ellipsis if they do not fit. */
function LevelUpWords({ level, message, still, lead, onDone }: { level: number; message: string; still: boolean; lead: string; onDone: () => void }) {
  const boxRef = useRef<HTMLDivElement>(null);
  const textRef = useRef<HTMLSpanElement>(null);
  const [run, setRun] = useState<{ from: number; to: number; ms: number } | null>(null);
  useLayoutEffect(() => {
    if (still) return;
    const box = boxRef.current;
    const text = textRef.current;
    if (!box || !text) return;
    const from = box.clientWidth;
    const to = -text.scrollWidth;
    const ms = Math.min(WORDS_MAX_MS, Math.max(WORDS_MIN_MS, ((from - to) / WORDS_PX_PER_S) * 1000));
    setRun({ from, to, ms });
  }, [still, message, level]);
  const words = (
    <>
      <span className="hype-train-words-lead">Level {level}!</span> {message}
    </>
  );
  if (still) {
    // Centered when they fit; otherwise they start at the left and end in an
    // ellipsis, so "Level n!" is always whole.
    return (
      <div className="absolute inset-0 flex items-center justify-center overflow-hidden">
        <span className={`min-w-0 max-w-full truncate text-[12.5px] font-extrabold ${lead} text-textPrimary`}>{words}</span>
      </div>
    );
  }
  return (
    <div ref={boxRef} className="hype-train-words absolute inset-0 overflow-hidden" aria-live="polite">
      <span
        ref={textRef}
        className={`absolute left-0 top-0 whitespace-nowrap text-[12.5px] font-extrabold ${lead} text-textPrimary ${
          run ? 'animate-hype-marquee' : 'invisible'
        }`}
        style={
          run
            ? ({
                '--hype-from': `${run.from}px`,
                '--hype-to': `${run.to}px`,
                '--hype-dur': `${run.ms}ms`,
              } as CSSProperties)
            : undefined
        }
        onAnimationEnd={(e) => {
          if (e.animationName === 'hype-marquee') onDone();
        }}
      >
        {words}
      </span>
    </div>
  );
}
