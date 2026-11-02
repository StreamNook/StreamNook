// The one-open-strip registry and the strip's placement and close math, kept
// free of React and the DOM so they are tested in node (the suite has no DOM).

export interface StripOwner {
  readonly token: object;
  close(): void;
}

let active: StripOwner | null = null;

/** Makes `owner` the open strip, closing whichever was open before. */
export function openStrip(owner: StripOwner): void {
  if (active && active.token !== owner.token) active.close();
  active = owner;
}

/** Forgets `token` if it is the open strip (its close or its unmount). */
export function releaseStrip(token: object): void {
  if (active?.token === token) active = null;
}

export function activeStripToken(): object | null {
  return active?.token ?? null;
}

export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** The chip has scrolled out of its list's visible box. */
export function chipLeftView(chip: Box, view: Box): boolean {
  return chip.bottom <= view.top || chip.top >= view.bottom || chip.right <= view.left || chip.left >= view.right;
}

export interface StripPlace {
  left: number | null;
  right: number | null;
  top: number;
  maxWidth: number;
  /** 1 opens rightward from the chip, -1 leftward. */
  dir: 1 | -1;
}

/**
 * Where the strip sits: beside the chip, never over it (so the chip stays
 * clickable), centred on the chip's middle. It opens toward the side with room
 * for `needed` px (rightward when it fits or when the right has at least as
 * much room); `lockDir` keeps the side chosen at open while it rides its row.
 */
export function stripPlace(chip: Box, windowWidth: number, needed: number, lockDir?: 1 | -1): StripPlace {
  const top = (chip.top + chip.bottom) / 2;
  const roomRight = windowWidth - chip.right - 10;
  const roomLeft = chip.left - 10;
  const dir: 1 | -1 = lockDir ?? (roomRight >= needed || roomRight >= roomLeft ? 1 : -1);
  if (dir === 1) return { left: chip.right + 2, right: null, top, maxWidth: Math.max(40, roomRight), dir };
  return { left: null, right: windowWidth - chip.left + 2, top, maxWidth: Math.max(40, roomLeft), dir };
}
