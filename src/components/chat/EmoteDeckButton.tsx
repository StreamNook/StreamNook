// The emote button as a deck (Brandon's pick, 2026-11-13): its face is the
// chat's emote of the moment (else the swapping smiley), and on hover or
// keyboard focus the deck's other cards (the GIF chip, the slash card) fan out
// from behind it, so the button says what its menu holds without adding a
// control. The face turns to the slash card or the GIF chip while the box is
// about commands or a GIF. Chat's composer and Settings > Chat's preview draw
// it; the phone draws `DeckFace` in its own button (no hover there).
//
// Motion is transform and opacity only. The fan is CSS (`.sn-deck-card` in
// globals.css, with its Motion and Performance rules); the face swap is
// framer-motion, instant under Motion Off and quiet modes. An animated emote
// draws its first frame wherever chat's emotes do.
import { AnimatePresence, motion } from 'framer-motion';
import { X } from 'phosphor-react';
import { useState, type CSSProperties, type Ref } from 'react';
import { getAppleEmojiUrl } from '../../services/emojiService';
import { useAppStore } from '../../stores/AppStore';
import { useLowPower, useStillChat } from '../../utils/lowPower';
import { staticEmoteUrl } from '../../utils/staticEmoteUrl';
import type { MomentEmote } from '../../hooks/useEmoteMoment';
import { Tooltip } from '../ui/Tooltip';
import type { DeckCard } from './composerHints';

const EASE = [0.2, 0.8, 0.2, 1] as const;

type ArtSize = 'inset' | 'phone' | 'card';

/** Whether the face draws an emote's first frame: Freeze emotes / Battery
 *  Saver, or Animate emotes set to anything but always (the face has no row
 *  to hover). */
function useStillFace(): boolean {
  const still = useStillChat();
  const animate = useAppStore((s) => s.settings.chat_design?.animate_emotes ?? 'always');
  const motion = useAppStore((s) => s.settings.motion_mode ?? 'full');
  // A loop in the composer is decoration: reduced or no motion stills it too.
  return still || animate !== 'always' || motion !== 'full';
}

/** The smiley, as chat draws it there: Apple art on desktop, the system glyph
 *  on the phone (as its emoji panel draws them). */
function Smiley({ smiley, size }: { smiley: string; size: ArtSize }) {
  if (size === 'phone') return <span className="text-[20px] leading-none">{smiley}</span>;
  const box = size === 'card' ? 'h-[11px] w-[11px]' : 'h-4 w-4';
  return <img src={getAppleEmojiUrl(smiley)} alt="" draggable={false} className={`${box} object-contain`} />;
}

/** The emote of the moment: its first frame where chat's emotes are still,
 *  the file itself when the CDN has no first frame (a still emote), and the
 *  smiley if the file cannot load at all. */
function MomentImage({ moment, smiley, size, still }: { moment: MomentEmote; smiley: string; size: ArtSize; still: boolean }) {
  const wanted = still ? staticEmoteUrl(moment.url) : moment.url;
  const [failed, setFailed] = useState<{ url: string; steps: number } | null>(null);
  const steps = failed?.url === moment.url ? failed.steps : 0;
  if (steps >= 2 || (steps === 1 && wanted === moment.url)) return <Smiley smiley={smiley} size={size} />;
  const src = steps === 1 ? moment.url : wanted;
  const box = size === 'phone' ? 'h-[22px] max-w-[30px]' : size === 'card' ? 'h-[11px] max-w-[11px]' : 'h-4 max-w-[22px]';
  return (
    <img
      src={src}
      alt=""
      draggable={false}
      onError={() => setFailed({ url: moment.url, steps: steps + 1 })}
      className={`${box} w-auto object-contain`}
    />
  );
}

/** One deck member's art. `inset` is the desktop composer's 16 px slot,
 *  `phone` the phone bar's 22 px one, `card` a fanned card's face. */
export function DeckArt({
  card,
  moment,
  smiley,
  size,
  still,
}: {
  card: DeckCard;
  moment: MomentEmote | null;
  smiley: string;
  size: ArtSize;
  still: boolean;
}) {
  if (card === 'slash') return <span className="sn-deck-chip sn-deck-slash" data-size={size}>/</span>;
  if (card === 'gif') return <span className="sn-deck-chip sn-deck-gif" data-size={size}>GIF</span>;
  if (moment) return <MomentImage moment={moment} smiley={smiley} size={size} still={still} />;
  return <Smiley smiley={smiley} size={size} />;
}

/** The face in front: a soft cross-fade between faces, instant where motion
 *  is off or the window is quieted. */
export function DeckFace({
  face,
  moment,
  smiley,
  size,
  className = '',
}: {
  face: DeckCard;
  moment: MomentEmote | null;
  smiley: string;
  size: 'inset' | 'phone';
  className?: string;
}) {
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
  const quiet = useLowPower();
  const still = useStillFace();
  const key = face === 'moment' ? (moment ? `m:${moment.url}` : `s:${smiley}`) : face;
  return (
    <span className={`relative grid place-items-center ${className}`}>
      <AnimatePresence initial={false}>
        <motion.span
          key={key}
          className="col-start-1 row-start-1 grid place-items-center"
          initial={{ opacity: 0, scale: 0.6 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: 0.6 }}
          transition={motionOff || quiet ? { duration: 0 } : { duration: 0.18, ease: EASE }}
        >
          <DeckArt card={face} moment={moment} smiley={smiley} size={size} still={still} />
        </motion.span>
      </AnimatePresence>
    </span>
  );
}

/** Where each fanned card comes to rest, by how many fan: lifted clear above
 *  the face, spread so neither covers the other's mark. A lone card rises
 *  straight up (off to one side it would read as a corner badge). */
const FAN: Record<number, string[]> = {
  1: ['translate(0px, -19px) rotate(6deg)'],
  2: ['translate(-10px, -17px) rotate(-13deg)', 'translate(10px, -17px) rotate(13deg)'],
};

/** The desktop emote button: the deck's face, with its other cards behind. */
export function EmoteDeckButton({
  open,
  face,
  cards,
  moment,
  smiley,
  label,
  buttonRef,
  onClick,
  onMouseLeave,
  onPointerOver,
  onFocusChange,
  // Hovered or focused, it rises over the send-as picker beside it, so the
  // fanned cards are never painted under it.
  className = 'absolute left-1 top-1/2 -translate-y-1/2 z-10 hover:z-20 focus-visible:z-20 w-7 h-7',
}: {
  open: boolean;
  face: DeckCard;
  /** The cards that fan on hover (`fanCards`). */
  cards: readonly DeckCard[];
  moment: MomentEmote | null;
  smiley: string;
  label: string;
  buttonRef?: Ref<HTMLButtonElement>;
  onClick?: () => void;
  onMouseLeave?: () => void;
  /** The pointer came over the button (the hints hold still while it is). */
  onPointerOver?: () => void;
  onFocusChange?: (focused: boolean) => void;
  /** Placement and size; the default is the inset at the field's left. */
  className?: string;
}) {
  const fan = FAN[cards.length] ?? [];
  const still = useStillFace();
  // While cards fan, they are the label (and would sit under a tooltip over
  // the button); the button keeps its name for screen readers either way.
  const fans = !open && fan.length > 0;
  return (
    <Tooltip content={open ? 'Close' : label} side="top" disabled={fans}>
      <button
        ref={buttonRef}
        type="button"
        onClick={onClick}
        onMouseLeave={onMouseLeave}
        onPointerEnter={onPointerOver}
        onFocus={() => onFocusChange?.(true)}
        onBlur={() => onFocusChange?.(false)}
        aria-label={open ? 'Close' : label}
        className={`sn-deck group flex items-center justify-center text-textSecondary hover:text-textPrimary transition-colors duration-200 ${className}`}
      >
        {open ? (
          <X size={16} weight="bold" aria-hidden className="transition-all duration-200 text-accent group-hover:drop-shadow-[0_0_5px_color-mix(in_srgb,var(--color-accent-neon)_80%,transparent)]" />
        ) : (
          <>
            {cards.slice(0, fan.length).map((card, i) => (
              <span
                key={card}
                aria-hidden
                className="sn-deck-card"
                style={{ '--sn-fan': fan[i], transitionDelay: i > 0 ? '40ms' : undefined } as CSSProperties}
              >
                <DeckArt card={card} moment={moment} smiley={smiley} size="card" still={still} />
              </span>
            ))}
            <DeckFace
              face={face}
              moment={moment}
              smiley={smiley}
              size="inset"
              className="z-[1] w-[22px] h-4 group-hover:drop-shadow-[0_0_5px_color-mix(in_srgb,var(--color-accent-neon)_80%,transparent)]"
            />
          </>
        )}
      </button>
    </Tooltip>
  );
}
