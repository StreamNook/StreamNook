// Whether your wall's slot on the You tab shows the wall, holds invisible space
// of a collapsed wall's height, or takes no room. The same rule as the page plate's slot
// (plateSlot): a wall shows; otherwise, once the answer has come, nothing; until
// then the slot is held while the member list has not loaded or you are a
// member. A wall saved on disk and painted first proves membership too (it
// arrives as a wall).
import type { PlateSlot } from '../../components/profile/profileLink';

export function ownWallSlot(
  registryLoaded: boolean,
  isMember: boolean,
  wall: object | null,
  answered: boolean,
): PlateSlot {
  if (wall) return 'shown';
  if (answered) return 'hidden';
  return !registryLoaded || isMember ? 'reserved' : 'hidden';
}
