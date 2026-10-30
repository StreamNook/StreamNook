// Every sentence the phones show about live alerts: the Which channels detail,
// the sheet's intro and Copy from Twitch's result line. Pure, so the wording is
// tested in one place.
import type { CopyAnswer, CopyCode } from './notifyChannels';

const INTRO_ALL = 'Alerting for every channel you follow. Turn off any you would rather not hear about.';

function plural(n: number, one: string, many: string): string {
  return n === 1 ? one : many;
}

function count(n: number): string {
  return n.toLocaleString('en-US');
}

/** The Which channels row's detail. `mutedCount` is Rust's count of muted
 *  channels you follow: undefined while it loads, null when the follow list
 *  cannot be read; `rawLength` is the stored list's length. */
export function whichChannelsDetail(mutedCount: number | null | undefined, rawLength: number): string {
  if (mutedCount === undefined) {
    return rawLength > 0 ? `Everything you follow except ${count(rawLength)}` : 'Everything you follow';
  }
  if (mutedCount === null) {
    return rawLength > 0
      ? `${count(rawLength)} ${plural(rawLength, 'channel', 'channels')} muted`
      : 'Everything you follow';
  }
  return mutedCount > 0 ? `Everything you follow except ${count(mutedCount)}` : 'Everything you follow';
}

/** The sheet's opening line, from the same numbers. */
export function sheetIntro(mutedCount: number | null | undefined, rawLength: number): string {
  if (mutedCount === undefined || mutedCount === null) {
    if (rawLength === 0) return INTRO_ALL;
    return mutedCount === undefined
      ? `Alerting for every channel you follow except ${count(rawLength)}.`
      : `${count(rawLength)} ${plural(rawLength, 'channel is', 'channels are')} muted.`;
  }
  return mutedCount > 0 ? `Alerting for every channel you follow except ${count(mutedCount)}.` : INTRO_ALL;
}

/** What one Copy from Twitch did. */
export function copyResultText(answer: Pick<CopyAnswer, 'added' | 'truncated'>): string {
  const n = answer.added.length;
  const head =
    n > 0
      ? `Muted ${count(n)} ${plural(n, 'channel', 'channels')} whose Twitch bell is off.`
      : 'Nothing new to copy. Every channel whose Twitch bell is off is already muted.';
  return answer.truncated ? `${head} Only your first 2,000 follows were checked.` : head;
}

/** Why a Copy or an Undo did nothing. */
export function copyErrorText(code: CopyCode): string {
  switch (code) {
    case 'signed-out':
      return 'Sign in to Twitch to copy your bells.';
    case 'wrong-account':
      return 'Twitch is signed in as another account here. Switch back to copy your bells.';
    case 'busy':
      return 'Already copying your bells.';
    case 'stale':
      return "That copy can't be undone anymore.";
    case 'save-failed':
      return "Couldn't save that. Try again.";
    case 'twitch-error':
    default:
      return "Twitch didn't answer. Try again in a moment.";
  }
}

/** What an Undo did. */
export function undoResultText(removed: number): string {
  return removed > 0
    ? `Alerts are back on for ${count(removed)} ${plural(removed, 'channel', 'channels')}.`
    : 'Nothing left to undo.';
}
