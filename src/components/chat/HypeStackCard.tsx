import { useEffect, type ReactNode } from 'react';
import HypeTrainBanner from '../HypeTrainBanner';
import { StackCard } from './ChatOverlayStack';
import type { StackCardControl, StackKind } from './chatTopStack';
import type { HypeTrainData } from '../../types';

/** The card's identity while a train runs: every running train's id. */
export const hypeStackId = (trains: readonly HypeTrainData[]): string | null =>
  trains.length > 0 ? trains.map((t) => t.id).join('|') : null;

/**
 * The hype train as the first card of the top-of-chat stack: one strip per
 * train (the combined feed can run several), above every other card. It has
 * one form, so a form change never remounts its level, its words or its clock;
 * the banner stays mounted even when Rust folds it into "N more cards" or the
 * view is not chat (Viewers, Mod Room, search), so it still expires and
 * levels up. The level-up confetti and its words are the banner's own.
 */
export function HypeStackCard({
  trains,
  stack,
  measure,
  compact = false,
  onLevelUp,
  onExpire,
  confettiTarget,
  renderLabel,
}: {
  trains: readonly HypeTrainData[];
  stack: StackCardControl;
  measure?: (kind: StackKind, form: 'strip' | 'open' | 'line', height: number) => void;
  compact?: boolean;
  onLevelUp?: (golden: boolean, colors: string[]) => void;
  onExpire?: (train: HypeTrainData) => void;
  confettiTarget?: HTMLElement | null;
  /** A line above a train's strip (the combined feed names whose train it is). */
  renderLabel?: (train: HypeTrainData) => ReactNode;
}) {
  const id = hypeStackId(trains);
  const onPresence = stack.onPresence;
  useEffect(() => {
    onPresence(id);
  }, [onPresence, id]);
  useEffect(() => () => onPresence(null), [onPresence]);
  if (trains.length === 0) return null;
  return (
    <StackCard kind="hype" control={stack} measure={measure} className="gap-1">
      {trains.map((train) => (
        <div key={train.id} className="flex flex-col">
          {renderLabel?.(train)}
          <HypeTrainBanner
            train={train}
            floating
            inStack
            compact={compact}
            onLevelUp={onLevelUp}
            onExpire={onExpire ? () => onExpire(train) : undefined}
            confettiTarget={onLevelUp ? undefined : confettiTarget}
          />
        </div>
      ))}
    </StackCard>
  );
}

export default HypeStackCard;
