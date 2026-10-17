// The steady tint a mention or reply to you keeps when Keep tinted is on
// (chat_design.mention_hold), and that a phrase, user or badge highlight keeps
// when its own rule has Keep tinted on. Chat rows and the settings preview both draw it
// from here, at the Highlight Appearance opacity phrase and event highlights
// use, so every highlighted row reads at one strength.

/** A left-to-right wash in `color` (any CSS color), `opacityPct` strong at
 *  the edge and half that through the middle, clear at the far end. */
export function mentionHoldBackground(color: string, opacityPct: number): string {
  const pct = Math.max(0, Math.min(100, opacityPct));
  const half = Math.round(pct / 2);
  return `linear-gradient(to right, color-mix(in srgb, ${color} ${pct}%, transparent), color-mix(in srgb, ${color} ${half}%, transparent), transparent)`;
}
