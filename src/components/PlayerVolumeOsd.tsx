import { AnimatePresence, motion } from 'framer-motion';
import { Volume1, Volume2, VolumeX } from 'lucide-react';
import type { VolumeOsdState } from '../hooks/useVolumeOsd';

// The readout that appears when volume is changed by mouse rather than by the
// control bar. Plyr's controls auto-hide, so without this a scroll gesture gives
// no feedback at all about where the volume landed.
//
// The `player-volume-osd` class is load-bearing: globals.css lifts it above
// Plyr's fullscreen element, which is fixed at z-index 10000000 and would
// otherwise paint straight over this.

/**
 * The level bar's colour, so the reading is legible at a glance before you've
 * read the number. Mixed from theme tokens rather than fixed hues, so it follows
 * a custom accent and stays correct on every theme.
 *
 * Three stages, each ending on a colour that's plainly different from the last,
 * so the change is readable on a 6px bar rather than being a saturation shift
 * you have to squint at:
 *
 *   0%  desaturated accent (near silence looks near-silent)
 *   40% the accent itself
 *   72% full warning amber
 *  100% full error red
 */
function levelColor(level: number): string {
  if (level <= 0.4) {
    const t = level / 0.4;
    return `color-mix(in srgb, var(--color-accent) ${40 + t * 60}%, var(--color-text-muted))`;
  }
  if (level <= 0.72) {
    const t = (level - 0.4) / 0.32;
    return `color-mix(in srgb, var(--color-warning) ${t * 100}%, var(--color-accent))`;
  }
  const t = (level - 0.72) / 0.28;
  return `color-mix(in srgb, var(--color-error) ${t * 100}%, var(--color-warning))`;
}

interface PlayerVolumeOsdProps {
  osd: VolumeOsdState | null;
  /**
   * Audio Boost's makeup-gain multiplier, or 1 when it's off. The reading
   * becomes the effective level (70% at a 1.5x boost reads 105%), because the
   * player's own 100% stops meaning "as loud as it goes" the moment boost is on.
   */
  boost?: number;
}

export function PlayerVolumeOsd({ osd, boost = 1 }: PlayerVolumeOsdProps) {
  const level = osd ? Math.min(1, Math.max(0, osd.volume)) : 0;
  const muted = osd?.muted ?? false;
  const gain = Number.isFinite(boost) && boost > 0 ? boost : 1;
  const boosted = Math.abs(gain - 1) > 0.01;

  // What you hear, which is what the number reports.
  const effective = level * gain;
  const percent = Math.round(effective * 100);

  // The bar stays on the player's own 0-100, so it always answers to the wheel.
  // Its COLOUR is keyed to the effective level instead, so a boosted stream runs
  // hot earlier — which is the honest reading, since a 1.5x boost really is at
  // full loudness by 67%.
  const barFill = Math.max(level * 100, 1);
  const barColor = muted ? 'var(--color-text-muted)' : levelColor(Math.min(1, effective));
  // Garnish, deliberately: the bar picks up a faint bloom as it gets loud, and
  // none at all when muted or quiet.
  const glow = muted ? 0 : Math.min(1, effective) * 0.55;

  const Icon = muted ? VolumeX : effective <= 0.5 ? Volume1 : Volume2;

  return (
    <div className="player-volume-osd pointer-events-none absolute inset-0 z-[70] flex items-center justify-center">
      <AnimatePresence>
        {osd && (
          // One stable key for the whole visible run. Keying on the value would
          // remount the panel on every notch, leaving the outgoing copy mid-exit
          // beside the incoming one.
          <motion.div
            key="volume-osd"
            initial={{ opacity: 0, scale: 0.96, y: 4 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.98, y: 2 }}
            transition={{ duration: 0.16, ease: [0.2, 0.8, 0.2, 1] }}
            // Fixed width, so the panel never resizes or drifts off the centre
            // line as the reading crosses 9% to 10% to 100%. It gives way on a
            // small MultiNook tile rather than overhanging it.
            className="liquid-glass-panel w-[208px] max-w-[80%] rounded-2xl px-4 py-3"
          >
            {/* Muted swaps the number for the word rather than adding a caption
                line, so the panel keeps one height and never wobbles when you
                middle-click mid-scroll. */}
            <div className="flex items-center justify-between gap-3">
              <Icon
                className="h-[19px] w-[19px] shrink-0 transition-colors duration-200"
                style={{ color: muted ? 'var(--color-text-muted)' : barColor }}
              />
              {muted ? (
                <span className="text-[22px] font-semibold leading-none tracking-tight text-textMuted">
                  Muted
                </span>
              ) : (
                <span className="flex items-baseline gap-1.5">
                  {/* Says why the reading can exceed 100 and that the extra isn't
                      something the wheel can reach. Only when boost is actually
                      doing something. */}
                  {boosted && (
                    <span
                      className="self-center rounded-md px-1.5 py-0.5 text-[10px] font-semibold leading-none tabular-nums"
                      style={{
                        color: 'var(--color-accent)',
                        backgroundColor: 'color-mix(in srgb, var(--color-accent) 16%, transparent)',
                      }}
                    >
                      {gain.toFixed(2).replace(/\.?0+$/, '')}x
                    </span>
                  )}
                  <span className="flex items-baseline gap-0.5">
                    <span className="text-[26px] font-semibold leading-none tabular-nums tracking-tight text-textPrimary">
                      {percent}
                    </span>
                    <span className="text-[13px] font-medium leading-none text-textSecondary">%</span>
                  </span>
                </span>
              )}
            </div>

            {/* Level bar. Width and colour both animate, so a run of notches
                reads as one continuous movement instead of a series of jumps.
                The track is mixed from a text token so it stays visible on light
                themes, where a white overlay would disappear. */}
            <div
              className="mt-3 h-[6px] w-full overflow-hidden rounded-full"
              style={{ backgroundColor: 'color-mix(in srgb, var(--color-text-muted) 28%, transparent)' }}
            >
              <div
                className="h-full rounded-full"
                style={{
                  width: `${barFill}%`,
                  backgroundColor: barColor,
                  boxShadow: glow > 0 ? `0 0 10px color-mix(in srgb, ${barColor} ${glow * 100}%, transparent)` : 'none',
                  transition: 'width 140ms cubic-bezier(0.2, 0.8, 0.2, 1), background-color 220ms ease, box-shadow 220ms ease',
                }}
              />
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

export default PlayerVolumeOsd;
