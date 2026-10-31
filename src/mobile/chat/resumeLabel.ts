/** What the resume circle shows once messages arrived while paused: "+1" up to
 *  "+99", then "99+". Never more than three characters, so it fits the circle;
 *  empty before anything arrived (the chevron shows). The iPhone's
 *  ChatListView says the same. */
export function resumeCountText(count: number): string {
  const n = Math.floor(count);
  if (!(n > 0)) return '';
  return n > 99 ? '99+' : `+${n}`;
}

/** How often the shown count may step, so each roll lands before the next one
 *  starts in fast chat. */
export const RESUME_COUNT_STEP_MS = 160;

/** What VoiceOver and TalkBack read for a paused chat's resume button. */
export function resumeLabel(count: number): string {
  if (count <= 0) return 'Scroll to the latest messages';
  return `${count} new ${count === 1 ? 'message' : 'messages'}, scrolls to the latest messages`;
}
