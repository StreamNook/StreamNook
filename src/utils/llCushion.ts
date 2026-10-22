// The live player's learned cushion, shared by the desktop and phone players.

/** A stall whose buffered media resumes within this many seconds past the
 *  playhead is a SEAM (a segment whose tail never arrived, so its successor
 *  starts a few frames later), not a delivery drought. Observed seams run
 *  70 ms to 1.8 s; a whole missing 2 s segment is a drought and stays out. */
export const SEAM_MAX_SECS = 1.95;

// Per-channel learned cushion (LL path), relative to the viewer's chosen target. The
// `base` is the user's target latency converted to the real (raw) cushion. The
// stall-adaptive cushion proves a channel's delivery wobbliness during a session;
// remembering it means a wobbly channel pays its stalls ONCE ever instead of once per
// session, while clean channels keep the user's tight target. The learned value relaxes
// by 0.5s per day, so one bad night doesn't tax a channel forever, and it is clamped to
// [base, base + 2] so it never drops below the user's target nor runs away above it.
// Version the store: when a stall-causing pipeline bug is fixed, cushions learned from
// those stalls are poisoned lessons; bump to orphan and re-learn.
// Namespaced by PLATFORM: a Kick "xqc" and a Twitch "xqc" are different streams
// with different segment lengths, and a shared key would hand one the other's
// learned cushion.
export const llCushionKey = (channel: string, provider = 'twitch') =>
  `streamnook.ll-cushion.v4.${provider}.${channel.toLowerCase()}`;

export function learnedLLCushion(
  channel: string | undefined,
  base: number,
  provider = 'twitch',
  headroom = 2,
): number {
  if (!channel) return base;
  try {
    const raw = localStorage.getItem(llCushionKey(channel, provider));
    if (!raw) return base;
    const { c, t } = JSON.parse(raw) as { c: number; t: number };
    if (!Number.isFinite(c) || !Number.isFinite(t)) return base;
    // Decay back toward the base half a second per day, so a channel that was
    // once unstable is retried rather than permanently penalised.
    const days = Math.max(0, (Date.now() - t) / 86_400_000);
    return Math.max(base, Math.min(base + headroom, c - 0.5 * days));
  } catch {
    return base;
  }
}
export function rememberLLCushion(channel: string | undefined, cushion: number, provider = 'twitch') {
  if (!channel) return;
  try {
    localStorage.setItem(
      llCushionKey(channel, provider),
      JSON.stringify({ c: cushion, t: Date.now() }),
    );
  } catch {
    /* storage full/blocked: the session keeps its in-memory value */
  }
}
