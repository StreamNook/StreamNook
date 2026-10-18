import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { HypeTrainData } from '../types';
import ConfettiBurst from './ConfettiBurst';
import { Logger } from '../utils/logger';
import { formatShortCount } from '../utils/streamStats';
import { fitHeaderDensity } from './chat/fitHeaderDensity';

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
  /** Fired when the countdown reaches 0, so the owner can clear its own state.
   *  The main player chat clears the global hype train here; pane/blended
   *  pollers manage their own removal, so they can leave this unset. */
  onExpire?: () => void;
}

export default function HypeTrainBanner({ train, confettiTarget, onExpire }: HypeTrainBannerProps) {
  const [timeRemaining, setTimeRemaining] = useState('');
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
  // Both sides of the strip step down in place while they overflow (CSS
  // `.hype-train-level` / `.hype-train-need` say what each step drops).
  const [levelEl, setLevelEl] = useState<HTMLDivElement | null>(null);
  const [needEl, setNeedEl] = useState<HTMLDivElement | null>(null);
  useEffect(() => (levelEl ? fitHeaderDensity(levelEl, 1, 0) : undefined), [levelEl]);
  useEffect(() => (needEl ? fitHeaderDensity(needEl, 2, 0) : undefined), [needEl]);

  // Smooth countdown, updated every second locally. On hitting 0 it fires
  // onExpire (the owner clears the train) rather than waiting for the next poll.
  useEffect(() => {
    if (!train.expires_at) {
      setTimeRemaining('');
      expiresAtRef.current = null;
      return;
    }
    expiresAtRef.current = train.expires_at;
    const tick = () => {
      if (!expiresAtRef.current) return;
      const diffMs = Math.max(0, new Date(expiresAtRef.current).getTime() - Date.now());
      if (diffMs === 0) {
        onExpireRef.current?.();
        return;
      }
      const minutes = Math.floor(diffMs / 60000);
      const seconds = Math.floor((diffMs % 60000) / 1000);
      setTimeRemaining(minutes > 0 ? `${minutes}:${seconds.toString().padStart(2, '0')}` : `${seconds}s`);
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
      setIsLevelUpCelebration(true);
      setCelebrationId((id) => id + 1);
    } else if (!isLevelUpCelebration) {
      setDisplayedLevel(train.level);
    }
    // isLevelUpCelebration is read but intentionally omitted from deps: including
    // it would re-run this on celebration start/end and risk re-triggering.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [train.level, train.progress]);

  // End the celebration ~8s after it starts. Keyed on celebrationId so progress
  // ticks DURING the celebration don't reset this timer (only a fresh level-up,
  // which bumps celebrationId, restarts it).
  useEffect(() => {
    if (celebrationId === 0) return;
    const t = setTimeout(() => {
      setIsLevelUpCelebration(false);
      setDisplayedLevel(previousLevelRef.current);
    }, 8000);
    return () => clearTimeout(t);
  }, [celebrationId]);

  // Guard against NaN when goal is 0 or undefined.
  const percentage = train.goal > 0
    ? Math.min(Math.round((train.progress / train.goal) * 100), 100)
    : 0;
  const remaining = Math.max(0, train.goal - train.progress);
  const bitsNeeded = remaining; // 1 bit = 1 point
  const subsNeeded = Math.ceil(remaining / 500); // 1 Tier1 sub = 500 points
  const isGolden = train.is_golden_kappa;

  return (
    <>
      {/* Level-up confetti. Bursts up from the bar and falls the full height of
          the host container. Portalled so it covers the whole chat/feed. */}
      {isLevelUpCelebration && confettiTarget &&
        createPortal(<ConfettiBurst key={celebrationId} golden={!!isGolden} />, confettiTarget)}

      <div className="relative h-9 overflow-hidden rounded-md mt-2 pointer-events-none">
        {/* Frosted track spanning the whole bar, under everything. Gives the
            level, the remaining count and the timer a surface to sit on
            instead of floating over raw chat. */}
        <div className="absolute inset-0 hype-train-track" />
        {/* Progress fill. Its wavy leading edge is a MASK on this wrapper, so
            nothing is ever painted over the fill and there is no band to see
            (globals.css explains why that matters and how the animation stays
            a cheap transform). The gradient sits on the child because it runs
            its own flow animation, and the wave animation would otherwise
            overwrite it. */}
        <div
          className="absolute left-0 hype-train-fill-wave"
          style={{ width: `${percentage}%`, transition: 'width 0.5s ease-out' }}
        >
          <div
            className={`absolute inset-0 ${
              isGolden ? 'hype-train-progress-golden' : 'hype-train-progress-rainbow'
            }`}
          />
        </div>
        {/* Celebration content over the whole strip */}
        {isLevelUpCelebration && (
          <div className="absolute inset-0 flex items-center justify-center z-20 pointer-events-none">
            {/* White flash (the confetti rains over the whole host container) */}
            <div className="absolute inset-0 bg-white/40 animate-hype-flash" />
            {/* Scrolling HYPE text */}
            <div className="animate-hype-marquee whitespace-nowrap">
              <span className="text-xl font-black text-white drop-shadow-glow mx-4">
                🎉 LEVEL UP! {celebrationMessage} 🎉 LEVEL UP! {celebrationMessage} 🎉
              </span>
            </div>
          </div>
        )}

        {/* Level (left), percentage (middle) and remaining/time (right) share
            one row. The equal side columns keep the percentage centered on the
            bar, and neither side can run into it. */}
        <div className="absolute inset-0 grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-2 px-2.5 z-10">
          {/* Left side - train icon and level. Too narrow for both, the icon
              goes and the level stays on one line. */}
          <div ref={setLevelEl} className="hype-train-level flex min-w-0 items-center gap-1.5 overflow-hidden whitespace-nowrap">
            {isGolden ? (
              <span className="hype-train-icon text-lg">✨</span>
            ) : (
              <svg className="hype-train-icon w-5 h-5 shrink-0 text-white" viewBox="0 0 15 13" fill="none">
                <path fillRule="evenodd" clipRule="evenodd" d="M4.10001 0.549988H2.40001V4.79999H0.700012V10.75H1.55001C1.55001 11.6889 2.31113 12.45 3.25001 12.45C4.1889 12.45 4.95001 11.6889 4.95001 10.75H5.80001C5.80001 11.6889 6.56113 12.45 7.50001 12.45C8.4389 12.45 9.20001 11.6889 9.20001 10.75H10.05C10.05 11.6889 10.8111 12.45 11.75 12.45C12.6889 12.45 13.45 11.6889 13.45 10.75H14.3V0.549988H6.65001V2.24999H7.50001V4.79999H4.10001V0.549988ZM12.6 9.04999V6.49999H2.40001V9.04999H12.6ZM9.20001 4.79999H12.6V2.24999H9.20001V4.79999Z" fill="currentColor" />
              </svg>
            )}
            <span className="text-sm font-bold text-white drop-shadow-sm">
              LVL {displayedLevel || train.level}
            </span>
          </div>

          {/* Middle - the percentage, kept in place (invisible) under the
              celebration so the sides do not shift when it ends. */}
          <span className={`text-xl font-black text-white drop-shadow-lg tabular-nums ${isLevelUpCelebration ? 'invisible' : ''}`}>
            {percentage}%
          </span>

          {/* Right side - bits/subs remaining and time. Too narrow for all of
              it, the subs go first, then the bits; the timer always stays. */}
          <div ref={setNeedEl} className="hype-train-need flex min-w-0 overflow-hidden">
            <div className="ml-auto flex items-center gap-1 whitespace-nowrap">
              <span className="hype-train-left text-[10px] text-white/80 drop-shadow-sm tabular-nums">
                {remaining > 0 ? (
                  <>
                    {formatShortCount(bitsNeeded)} bits<span className="hype-train-subs"> / {subsNeeded} subs</span> left
                  </>
                ) : '🎉'}
              </span>
              <span className="hype-train-left text-[10px] text-white/50">|</span>
              <span className="text-[10px] text-white/70 drop-shadow-sm tabular-nums">
                {timeRemaining}
              </span>
            </div>
          </div>
        </div>
      </div>
    </>
  );
}
