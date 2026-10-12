// Pure layout decisions for the overlay renderer, kept out of OverlayChat.tsx
// so that file exports only components (fast refresh) and these stay testable.
// Synced to the hosted overlay alongside the renderer.

import type { MessageSegment } from '../../services/twitchChat';
import type { OverlayStyle } from './overlayConfig';
import { nameColorReadsLight } from '../../utils/nameColor';

/**
 * Which zero-width emotes sit on top of which emote. A zero-width emote (7TV's
 * overlay flag) draws over the emote or emoji before it, across at most one
 * whitespace-only run, the way chat clients stack them; several in a row all
 * land on the same base. One with nothing to sit on renders as a normal emote.
 * `attached` maps a base index to its overlay indices; `skip` holds the
 * overlays and the spaces they bridged, which render inside the stack instead.
 * `drawsImage` says whether an overlay will draw as a picture: one shown as its
 * typed word (a hidden personal emote) stays in the text flow, never on top.
 */
export function zeroWidthLayout(
  segs: MessageSegment[],
  drawsImage: (s: MessageSegment) => boolean = () => true,
): { attached: Map<number, number[]>; skip: Set<number> } {
  const attached = new Map<number, number[]>();
  const skip = new Set<number>();
  let base = -1;
  let gap = -1;
  segs.forEach((s, i) => {
    if (s.type === 'emote' && s.is_zero_width && base >= 0 && drawsImage(s)) {
      const list = attached.get(base) ?? [];
      list.push(i);
      attached.set(base, list);
      skip.add(i);
      if (gap >= 0) skip.add(gap);
      gap = -1;
      return;
    }
    if (s.type === 'text' && s.content.trim() === '' && base >= 0 && gap < 0) {
      gap = i;
      return;
    }
    const canCarry = s.type === 'emote' || s.type === 'emoji';
    base = canCarry ? i : -1;
    gap = -1;
  });
  return { attached, skip };
}

/**
 * Whether names sit on something dark: the bubble when bubbles are on, the panel
 * when the background is solid, otherwise the stream itself, which the text
 * shadow darkens unless the streamer picked a light shadow.
 */
export function overlayBackdropIsDark(style: OverlayStyle): boolean {
  const backdrop = style.bubble
    ? style.bubbleColor || '#0e0e10'
    : style.background === 'solid'
      ? style.backgroundColor
      : style.textShadow !== false
        ? style.textShadowColor || '#000000'
        : '#000000';
  return nameColorReadsLight(backdrop) !== true;
}

/**
 * Which line of a message a GIF starts on, 0 being the name's line, from the
 * boxes of everything drawn before it in the row and the GIF's own top.
 *
 * The GIF hangs from the top of its line (text-top), so a box whose middle is
 * above that top sits on an earlier line, and everything on the GIF's own line
 * (text, middle-aligned badges and emotes) has its middle below it. Boxes that
 * share a line overlap, so they are counted once: a new line starts where a
 * box's middle falls below every box seen so far on the current one.
 */
export function gifLineIndex(before: ReadonlyArray<{ top: number; bottom: number }>, gifTop: number): number {
  const above = before
    .filter((r) => r.bottom > r.top && (r.top + r.bottom) / 2 < gifTop)
    .slice()
    .sort((a, b) => a.top - b.top);
  let lines = 0;
  let lineBottom = Number.NEGATIVE_INFINITY;
  for (const r of above) {
    if ((r.top + r.bottom) / 2 > lineBottom) {
      lines++;
      lineBottom = r.bottom;
    } else {
      lineBottom = Math.max(lineBottom, r.bottom);
    }
  }
  return lines;
}

/** The highest Max lines the overlay applies, whatever a config says. */
export const MAX_MESSAGE_LINES_CAP = 6;

/**
 * The line cap for a message row under Max lines (0 when Max lines is off).
 * A GIF too wide for what is left of the name's line wraps below it, and a
 * plain cap would then cut the GIF's whole line away. So a GIF's line always
 * counts as one line of the cap and the text clamps around it: the cap reaches
 * at least as far as the line the GIF starts on (`gifLine`, from
 * `gifLineIndex`; 0 for a row without one).
 */
export function messageLineClamp(maxMessageLines: number | undefined, gifLine = 0): number {
  const lines = Math.round(maxMessageLines ?? 0);
  if (!(lines >= 1)) return 0;
  return Math.max(Math.min(MAX_MESSAGE_LINES_CAP, lines), gifLine + 1);
}
