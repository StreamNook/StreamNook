import type { HypeTrainData } from '../types';

/** Which look a hype train wears: its fill, glow, timer ring, train and
 *  confetti colors. One field, so a new kind of train (Twitch's newer trains)
 *  is one more look here and in its CSS (`.hype-train-card[data-look]`), not a
 *  second flag threaded through the bar. The iPhone twin is SNKit
 *  `HypeTrainLook`. */
export type HypeTrainLook = 'standard' | 'golden';

export function hypeTrainLook(train: Pick<HypeTrainData, 'is_golden_kappa'>): HypeTrainLook {
  return train.is_golden_kappa ? 'golden' : 'standard';
}

/** The track fill's class for each look (globals.css). */
export const HYPE_LOOK_FILL: Record<HypeTrainLook, string> = {
  standard: 'hype-train-progress-rainbow',
  golden: 'hype-train-progress-golden',
};
