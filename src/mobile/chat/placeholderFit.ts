// The message field is one line tall, and a textarea placeholder that is too
// long wraps onto a second line the field never shows ("Slow mode (5" with the
// rest cut off), since Chromium ignores text-overflow on a placeholder. So the
// field says the longest wording that fits its width, else its shortest.

let measureCanvas: HTMLCanvasElement | null = null;

/** Width of `text` in `font` (a CSS font shorthand), or null when nothing can
 *  measure it (no canvas, no font yet). */
export function measureText(text: string, font: string): number | null {
  if (!font || typeof document === 'undefined') return null;
  measureCanvas ??= document.createElement('canvas');
  const ctx = measureCanvas.getContext('2d');
  if (!ctx) return null;
  ctx.font = font;
  return ctx.measureText(text).width;
}

/** The first wording (longest first) whose width fits, else the last one.
 *  `width` of 0 or less means the field is not measured yet: the first one. */
export function pickFittingText(
  wordings: readonly string[],
  width: number,
  widthOf: (text: string) => number | null,
): string {
  const list = wordings.filter((w) => w.length > 0);
  if (list.length === 0) return '';
  if (width <= 0) return list[0];
  for (const text of list) {
    const w = widthOf(text);
    if (w === null || w <= width) return text;
  }
  return list[list.length - 1];
}
