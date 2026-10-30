/** What VoiceOver and TalkBack read for a paused chat's resume button. */
export function resumeLabel(count: number): string {
  if (count <= 0) return 'Scroll to the latest messages';
  return `${count} new ${count === 1 ? 'message' : 'messages'}, scrolls to the latest messages`;
}
