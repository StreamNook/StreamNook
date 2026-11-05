// Which of the plate's three states a surface draws, and what a desktop surface
// asks Rust. Pure, so the rules are tested without a screen.
import type { PageLinkAnswer } from '../../types/profileView';

/** `hidden`: nothing, no space. `reserved`: an empty plate of the same size, so
 *  nothing moves when the link lands. `shown`: the plate. */
export type PlateSlot = 'hidden' | 'reserved' | 'shown';

/** What `streamnook_profile_link` is asked: the member's id and the member's
 *  own Twitch login for that id (never the login of the chat platform the card
 *  was opened from), or null while either is missing. */
export function profileLinkAsk(
  memberId: string | null | undefined,
  login: string | null | undefined,
): { memberId: string; login: string } | null {
  const id = (memberId ?? '').trim();
  const name = (login ?? '').trim();
  return id && name ? { memberId: id, login: name } : null;
}

/** One key per question, so an answer is only used for the member it was for. */
export function profileLinkKey(
  memberId: string | null | undefined,
  login: string | null | undefined,
): string | null {
  const ask = profileLinkAsk(memberId, login);
  return ask ? `${ask.memberId}|${ask.login}` : null;
}

export interface DesktopPlateInput {
  /** Rust's answer for this member, or null while it is on its way. */
  answer: PageLinkAnswer | null;
  /** Whether there was anything to ask yet (the login is known). */
  asked: boolean;
  /** The surface has finished loading what it can: nothing more will arrive. */
  loadFinished: boolean;
  /** The surface only exists for members (the member hero) or the viewer
   *  already knows the member is one; false means do not hold space. */
  expectMember: boolean;
}

/** The desktop's slot. A link shows. A "not a member" answer, or a surface that
 *  does not expect one, shows nothing. A member we are waiting on keeps its
 *  space until the answer lands, and gives it up once the surface has loaded and
 *  no link can come (not ready, or nothing to ask). */
export function desktopPlateSlot(input: DesktopPlateInput): PlateSlot {
  if (input.answer?.link) return 'shown';
  if (input.answer?.state === 'not_member') return 'hidden';
  if (!input.expectMember) return 'hidden';
  const settled = !input.asked || input.answer !== null;
  return input.loadFinished && settled ? 'hidden' : 'reserved';
}
