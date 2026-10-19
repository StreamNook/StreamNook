import { type ReactNode } from 'react';
import { ChevronDown } from 'lucide-react';
import { AnimatePresence, motion, useIsPresent } from 'framer-motion';
import { STACK_GAP_PX } from './chatTopStack';

// How a card leaves the stack: it fades where it is, then folds its height
// (and the gap after it) away, so the cards below slide up only once it is
// gone from sight. The curve is the other chat-top cards' ease-out.
const LEAVE_FADE_S = 0.2;
const LEAVE_FOLD_S = 0.2;
const LEAVE_EASE: [number, number, number, number] = [0.16, 1, 0.3, 1];

/**
 * Wraps one stack card so it plays its exit instead of vanishing. `onLeft`
 * is useStackPresence's release: the card's slot goes back to the stack only
 * once the fade and fold are done. Render it on every path (empty when the
 * card has nothing to show) so the same instance sees the card go.
 */
export function StackCardPresence({ onLeft, children }: { onLeft: () => void; children?: ReactNode }) {
  return <AnimatePresence onExitComplete={onLeft}>{children}</AnimatePresence>;
}

/**
 * The card shared by everything at the top of chat (poll, prediction, the
 * phone's drop progress), in its two forms:
 *
 * - the strip, one line: the card's glyph, its headline truncated, a short
 *   status (the countdown) and the chevron;
 * - open: the same line naming the card's kind instead, then the headline on
 *   its own line (two lines at most) and the body.
 *
 * Both forms share the head row at one height, so folding and opening never
 * shift the cards around it, and two strips fit the stack's smallest cap
 * (services/chat_top_stack.rs MIN_HEIGHT: 2 x 36 px + the 4 px gap).
 *
 * Positioning is not here: ChatOverlayStack (desktop) and the watch screen's
 * stack (phone) lay the cards out in the order Rust gives. Inside a capped
 * stack the open card shrinks and its body scrolls; the head never does.
 *
 * Motion preference needs no handling here. MotionScope wraps the app in a
 * framer MotionConfig, so this transition already respects Interface > Motion.
 */
interface OverlayBannerProps {
  /** The card's glyph, drawn in a 20 px slot at the head of the row. */
  icon: ReactNode;
  /** A different glyph for the open head, when the strip's (the drop's art)
   *  would repeat what the body shows. */
  openIcon?: ReactNode;
  /** What the card is ("Poll", "Prediction"), named on the head while open. */
  kind?: ReactNode;
  /** Headline: truncated on the strip, its own line (two at most) when open. */
  title: ReactNode;
  /** The short status at the end of the head (countdown, Locked, Final). */
  badges?: ReactNode;
  isExpanded: boolean;
  onToggleExpanded: () => void;
  /** Drawn along the strip's bottom edge while folded (the drop's progress). */
  stripLine?: ReactNode;
  /** Place in the stack (a flex `order`). */
  order?: number;
  /** Body, rendered only while expanded. */
  children?: ReactNode;
}

export function OverlayBanner({
  icon,
  openIcon,
  kind,
  title,
  badges,
  isExpanded,
  onToggleExpanded,
  stripLine,
  order,
  children,
}: OverlayBannerProps) {
  // A leaving card cannot be tapped while it fades.
  const present = useIsPresent();
  return (
    <motion.div
      initial={{ opacity: 0, y: -12, scale: 0.97 }}
      // Height and margin are named so a card that comes back mid-leave
      // unfolds again; instant, so a card that opens right after it appears
      // is never held at its strip's height.
      animate={{ opacity: 1, y: 0, scale: 1, height: 'auto', marginBottom: 0 }}
      transition={{
        type: 'spring',
        stiffness: 320,
        damping: 26,
        height: { duration: 0 },
        marginBottom: { duration: 0 },
      }}
      exit={{
        opacity: 0,
        height: 0,
        marginBottom: -STACK_GAP_PX,
        transition: {
          opacity: { duration: LEAVE_FADE_S, ease: LEAVE_EASE },
          height: { delay: LEAVE_FADE_S, duration: LEAVE_FOLD_S, ease: LEAVE_EASE },
          marginBottom: { delay: LEAVE_FADE_S, duration: LEAVE_FOLD_S, ease: LEAVE_EASE },
        },
      }}
      className={`flex flex-col min-h-0 ${isExpanded ? 'shrink' : 'shrink-0'} ${present ? '' : 'pointer-events-none'}`}
      style={order === undefined ? undefined : { order }}
    >
      <div className="relative flex flex-col min-h-0 bg-background rounded-lg border border-border shadow-md shadow-black/25 overflow-hidden">
        <button
          type="button"
          onClick={onToggleExpanded}
          aria-expanded={isExpanded}
          className="shrink-0 w-full h-[34px] flex items-center gap-2 pl-2.5 pr-2.5 text-left hover:bg-textPrimary/[0.04] active:bg-textPrimary/[0.06] transition-colors"
        >
          <span className="w-5 h-5 shrink-0 flex items-center justify-center">
            {isExpanded && openIcon ? openIcon : icon}
          </span>
          {isExpanded ? (
            <span className="flex-1 min-w-0 truncate text-[12px] font-medium text-textSecondary">{kind}</span>
          ) : (
            <span className="flex-1 min-w-0 truncate text-[13px] font-semibold text-textPrimary">{title}</span>
          )}
          {badges && <span className="shrink-0 flex items-center">{badges}</span>}
          <ChevronDown
            className={`w-4 h-4 shrink-0 text-textSecondary transition-transform duration-200 ${
              isExpanded ? 'rotate-180' : ''
            }`}
          />
        </button>
        {!isExpanded && stripLine}

        {isExpanded && (
          // In a capped stack the body scrolls under the head; its bottom edge
          // fades so a cut-off row reads as more below, not as the end. Every
          // body ends on at least the fade's height of padding, so a body that
          // fits loses nothing to it.
          <div className="min-h-0 overflow-y-auto overscroll-contain [mask-image:linear-gradient(to_bottom,#000_calc(100%-12px),transparent)]">
            {/* The padding sits outside the clamp: inside it, a third line
                showed through the padding under the ellipsis. */}
            <div className="px-3 pb-2.5">
              <p className="text-[14px] font-semibold leading-[18px] text-textPrimary line-clamp-2 break-words">
                {title}
              </p>
            </div>
            {children}
          </div>
        )}
      </div>
    </motion.div>
  );
}

/** The head's status text, quiet and centered on the head row: a countdown
 *  in monospace so its digits hold still, words in the card's own face. */
export function BannerStatus({
  children,
  tone = 'quiet',
  mono = true,
}: {
  children: ReactNode;
  tone?: 'quiet' | 'done';
  mono?: boolean;
}) {
  return (
    <span
      className={`text-[12px] leading-none tabular-nums ${mono ? 'font-mono' : 'font-medium'} ${
        tone === 'done' ? 'text-success font-semibold' : 'text-textSecondary'
      }`}
    >
      {children}
    </span>
  );
}

/**
 * One choice's share: its title and numbers on one line, then a thin bar in the
 * accent floored for cards, on a neutral track. No text sits on a colored fill,
 * so a pale accent on a tinted card still reads.
 */
export function ShareRow({
  title,
  mark,
  trailing,
  percent,
  strong = false,
  barClass = 'bg-accent-on-surface',
}: {
  title: ReactNode;
  /** The viewer's vote or the winner, before the title. */
  mark?: ReactNode;
  /** Votes and percent, at the end of the line. */
  trailing?: ReactNode;
  percent: number;
  /** The winner, whose title is set heavier. */
  strong?: boolean;
  /** The fill, when it is not the accent (a prediction side's own color). */
  barClass?: string;
}) {
  return (
    <>
      <span className="flex items-center gap-1.5 min-w-0">
        {mark && <span className="shrink-0 flex items-center">{mark}</span>}
        <span className={`flex-1 min-w-0 truncate text-[13px] text-textPrimary ${strong ? 'font-semibold' : 'font-medium'}`}>
          {title}
        </span>
        {trailing && <span className="shrink-0 flex items-center gap-2 text-[12px] tabular-nums">{trailing}</span>}
      </span>
      <span className="mt-1 block h-1 rounded-full bg-textPrimary/[0.12] overflow-hidden">
        <span
          className={`block h-full rounded-full ${barClass} transition-[width] duration-500`}
          style={{ width: `${percent}%` }}
        />
      </span>
    </>
  );
}

export default OverlayBanner;
