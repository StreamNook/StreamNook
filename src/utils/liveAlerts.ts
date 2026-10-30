// Desktop's door to a channel's live alerts.
//
// Every bell, menu row and list row sets the state it shows (`on`), never a
// blind toggle, through one chain, so presses apply in the order they were
// made whichever surface made them. Rust decides the stored list; this file
// only orders the calls and words the Copy from Twitch answers.

import { invoke } from '@tauri-apps/api/core';

export interface AlertChannelRef {
  provider: string;
  userId?: string | null;
  userLogin?: string | null;
}

let chain: Promise<unknown> = Promise.resolve();

/** Turn one channel's live alerts on or off. Answers the stored state (`null`: no identity). */
export function setLiveAlert(channel: AlertChannelRef, on: boolean): Promise<boolean | null> {
  const run = chain.then(() =>
    invoke<boolean | null>('set_live_alert', {
      provider: channel.provider,
      userId: channel.userId ?? '',
      userLogin: channel.userLogin ?? '',
      on,
    }),
  );
  chain = run.catch(() => undefined);
  return run;
}

/** Rust's answer to Copy from Twitch. */
export interface CopyAnswer {
  copy_id: number;
  added: string[];
  bells_off: number;
  already: number;
  skipped: number;
  truncated: boolean;
}

// The words below are the phones' (src/mobile/liveAlertsText.ts, the iPhone's
// LiveAlertText), so every app says the same thing.

const count = (n: number) => n.toLocaleString('en-US');
const plural = (n: number, one: string, many: string) => (n === 1 ? one : many);
const INTRO_ALL = 'Alerting for every channel you follow. Turn off any you would rather not hear about.';

/** The Which channels line. `mutedCount` is Rust's count of muted channels
 *  you follow: undefined while it loads (or the list is not read), null when
 *  the follow list cannot be read; `rawLength` is the stored list's length. */
export function whichChannelsDetail(mutedCount: number | null | undefined, rawLength: number): string {
  if (mutedCount === undefined) {
    return rawLength > 0 ? `Everything you follow except ${count(rawLength)}` : 'Everything you follow';
  }
  if (mutedCount === null) {
    return rawLength > 0 ? `${count(rawLength)} ${plural(rawLength, 'channel', 'channels')} muted` : 'Everything you follow';
  }
  return mutedCount > 0 ? `Everything you follow except ${count(mutedCount)}` : 'Everything you follow';
}

/** The list's opening line, from the same numbers. */
export function sheetIntro(mutedCount: number | null | undefined, rawLength: number): string {
  if (mutedCount === undefined || mutedCount === null) {
    if (rawLength === 0) return INTRO_ALL;
    return mutedCount === undefined
      ? `Alerting for every channel you follow except ${count(rawLength)}.`
      : `${count(rawLength)} ${plural(rawLength, 'channel is', 'channels are')} muted.`;
  }
  return mutedCount > 0 ? `Alerting for every channel you follow except ${count(mutedCount)}.` : INTRO_ALL;
}

/** The result line after a copy. */
export function copyResultText(answer: Partial<CopyAnswer> & Pick<CopyAnswer, 'added' | 'truncated'>): string {
  const n = answer.added.length;
  const head =
    n > 0
      ? `Muted ${count(n)} ${plural(n, 'channel', 'channels')} whose Twitch bell is off.`
      : 'Nothing new to copy. Every channel whose Twitch bell is off is already muted.';
  return answer.truncated ? `${head} Only your first 2,000 follows were checked.` : head;
}

const ERROR_TEXT: Record<string, string> = {
  'signed-out': 'Sign in to Twitch to copy your bells.',
  'wrong-account': 'Twitch is signed in as another account here. Switch back to copy your bells.',
  'twitch-error': "Twitch didn't answer. Try again in a moment.",
  busy: 'Already copying your bells.',
  stale: "That copy can't be undone anymore.",
  'save-failed': "Couldn't save that. Try again.",
};

/** The result line for a refused copy or undo. Unknown errors read as Twitch not answering. */
export function copyErrorText(err: unknown): string {
  const code = typeof err === 'string' ? err : '';
  return ERROR_TEXT[code] ?? ERROR_TEXT['twitch-error'];
}

/** The result line after an undo. */
export function undoResultText(removed: number): string {
  return removed > 0
    ? `Alerts are back on for ${count(removed)} ${plural(removed, 'channel', 'channels')}.`
    : 'Nothing left to undo.';
}
