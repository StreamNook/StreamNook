// The chat composer row (ChatWidget) compresses in place when the chat column
// is narrow: the tool buttons beside the field slim down so the field keeps a
// usable width (`.sn-composer-row[data-narrow]` in globals.css). Nothing moves
// to a second row. The flag is written straight onto the element, so a resize
// re-renders nothing. The field's prompt follows it the same way: Chromium
// ignores text-overflow on a textarea placeholder, so a full prompt was cut
// mid-word ("Send a mes") with no ellipsis.

/** Below this row width the tools compress. */
export const COMPOSER_NARROW_PX = 320;

/** Whether a composer row this wide compresses its tools. A hidden row (0 px)
 *  does not. */
export function isNarrowComposer(width: number): boolean {
  return width > 0 && width < COMPOSER_NARROW_PX;
}

/** Ref callback for the composer row: keeps `data-narrow` in step with the
 *  row's width, and returns the cleanup React runs when the row unmounts. */
export function trackComposerWidth(row: HTMLElement | null): (() => void) | undefined {
  if (!row || typeof ResizeObserver === 'undefined') return undefined;
  const apply = () => {
    if (isNarrowComposer(row.clientWidth)) row.dataset.narrow = '';
    else delete row.dataset.narrow;
    syncComposerPlaceholder(row);
  };
  apply();
  const observer = new ResizeObserver(apply);
  observer.observe(row);
  return () => observer.disconnect();
}

/** Puts the prompt that fits on the row's field: its short one
 *  (`data-placeholder-narrow`) while the row is narrow, else its full one
 *  (`data-placeholder-wide`). Runs on every width change, and from the
 *  composer whenever its prompt changes (a React render writes the full one
 *  back). */
export function syncComposerPlaceholder(row: HTMLElement): void {
  const field = row.querySelector<HTMLTextAreaElement>('textarea[data-placeholder-wide]');
  if (!field) return;
  const wide = field.dataset.placeholderWide ?? '';
  const want = row.dataset.narrow !== undefined ? field.dataset.placeholderNarrow || wide : wide;
  if (field.placeholder !== want) field.placeholder = want;
}
