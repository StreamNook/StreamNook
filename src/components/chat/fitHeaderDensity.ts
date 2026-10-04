// Compacts a header row in place instead of letting its last controls run past
// the edge, where the header clips them. The row's `data-density` steps from 0
// up to `max` until nothing overflows; CSS decides what each step gives up, the
// cheapest first. Steps below `lossless` also run while any `[data-fit-label]`
// inside the row is truncated, so free savings go to keeping the title whole.
// Returns the cleanup for the effect that started it.
export function fitHeaderDensity(row: HTMLElement, max: number, lossless: number): () => void {
  const overflows = () => row.scrollWidth > row.clientWidth + 1;
  const labelCut = () =>
    Array.from(row.querySelectorAll<HTMLElement>('[data-fit-label]')).some(
      (el) => el.scrollWidth > el.clientWidth + 1,
    );
  let frame = 0;
  const fit = () => {
    frame = 0;
    let level = 0;
    row.dataset.density = '0';
    while (level < max && (overflows() || (level < lossless && labelCut()))) {
      level += 1;
      row.dataset.density = String(level);
    }
  };
  // Batched to the next frame: fitting changes the very sizes being watched.
  const schedule = () => {
    if (!frame) frame = requestAnimationFrame(fit);
  };
  fit();
  const ro = new ResizeObserver(schedule);
  ro.observe(row);
  // Content changes too: a pin arriving, an unread chip, a linked platform.
  const mo = new MutationObserver(schedule);
  mo.observe(row, { childList: true, subtree: true, characterData: true });
  return () => {
    ro.disconnect();
    mo.disconnect();
    if (frame) cancelAnimationFrame(frame);
  };
}
