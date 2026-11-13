// What the app's own Stop controls send and say when Autopilot is the drops
// provider. Stop names the badge it shows, so only that badge is set aside, and
// while Missing badges is on the toast says Stop also paused it.

/** The one toast every Stop route shows while Missing badges is on. */
export const MISSING_BADGES_PAUSED_TOAST =
  'Stopped. Missing badges is paused until you resume it or StreamNook restarts.';

/** The args of a `drops.stop` action: the campaign the control shows, when it
 *  knows one. An older Autopilot ignores the key. */
export function stopArgs(campaignId?: string | null): { campaign_id?: string } {
  return campaignId ? { campaign_id: campaignId } : {};
}

/** The toast for a Stop: the route's own words, or the paused sentence while
 *  the Missing badges switch is on. */
export function stopToast(
  drops: { collect_missing_badges?: boolean } | null | undefined,
  plain: string,
): string {
  return drops?.collect_missing_badges ? MISSING_BADGES_PAUSED_TOAST : plain;
}
