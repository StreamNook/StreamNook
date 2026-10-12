import type { ChannelReward, RewardAvailability } from '../types';

/** Time left as "4:12", or "1:04:09" past an hour. Never negative. */
export const formatCountdown = (ms: number): string => {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
};

/**
 * Rust's words for a blocked reward, short (`label`, the list row) or full
 * (`detail`, the detail view), with a running cooldown's time left added.
 * Null when the reward is redeemable.
 */
export const rewardStatusText = (
  availability: RewardAvailability,
  nowMs: number,
  form: 'label' | 'detail',
): string | null => {
  if (availability.redeemable) return null;
  const words = (form === 'label' ? availability.label : availability.detail) ?? availability.label ?? null;
  if (words === null) return null;
  if (availability.reason === 'on_cooldown' && availability.cooldown_until_ms !== null) {
    return `${words} · ${formatCountdown(availability.cooldown_until_ms - nowMs)}`;
  }
  return words;
};

/** `v` to one decimal place, or whole from `wholeFrom` up, never with a trailing ".0". */
const shortNumber = (v: number, wholeFrom: number): string =>
  String(v >= wholeFrom ? Math.round(v) : Math.round(v * 10) / 10);

/** A cost for the narrow list pill: 950, 3k, 2.5k, 1.2M. */
export const formatCostShort = (cost: number): string => {
  // The rounded value picks the unit, so 999,999 reads "1M", never "1000k".
  if (cost >= 999_500) return `${shortNumber(cost / 1_000_000, 10)}M`;
  if (cost >= 1000) return `${shortNumber(cost / 1000, 100)}k`;
  return cost.toLocaleString();
};

/**
 * A fresh id for one redeem attempt: 32 lowercase hex characters, the form
 * Twitch's own client sends. The menu reuses it while the same message is
 * retried, so Twitch can tell a retry from a second redeem.
 */
export const newTransactionId = (): string => {
  const c = globalThis.crypto;
  if (c?.randomUUID) return c.randomUUID().replace(/-/g, '');
  const bytes = new Uint8Array(16);
  if (c?.getRandomValues) c.getRandomValues(bytes);
  else for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
};

/** What redeeming a reward involves, from its built-in type. */
export type RewardKind = 'highlight' | 'sub_message' | 'random_emote' | 'choose_emote' | 'modify_emote' | 'input' | 'standard';

export const rewardKind = (reward: Pick<ChannelReward, 'reward_type' | 'is_user_input_required'>): RewardKind => {
  switch (reward.reward_type) {
    case 'SEND_HIGHLIGHTED_MESSAGE':
      return 'highlight';
    case 'SINGLE_MESSAGE_BYPASS_SUB_MODE':
      return 'sub_message';
    case 'RANDOM_SUB_EMOTE_UNLOCK':
      return 'random_emote';
    case 'CHOSEN_SUB_EMOTE_UNLOCK':
      return 'choose_emote';
    case 'CHOSEN_MODIFIED_SUB_EMOTE_UNLOCK':
      return 'modify_emote';
    default:
      return reward.is_user_input_required ? 'input' : 'standard';
  }
};

/** Rewards whose redeem carries the viewer's typed text. */
export const rewardTakesText = (kind: RewardKind): boolean =>
  kind === 'highlight' || kind === 'sub_message' || kind === 'input';
