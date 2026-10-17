import type { RewardAvailability } from '../types';

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
