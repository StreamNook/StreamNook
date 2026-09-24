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
