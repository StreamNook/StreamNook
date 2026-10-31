// Close video (SN-37): the full player's X stops the stream and its chat carries
// on in Chats, the phones' chat-only screen. The stream's own room is kept first
// (Rust's chat tabs: a kept stream tab stays one of your chats when the player
// closes), then the player closes as it always does. Pure and injected, so the
// rule and the order are tested here; the iPhone's twin is SNKit `CloseVideo`,
// tested with the same cases.
import type { TwitchStream } from '../../types';
import { streamProvider } from '../../utils/streamProvider';

/** What the button reads, as the store holds it. */
export interface CloseVideoFacts {
  mini: boolean;
  pip: boolean;
  /** Cast or TV screen mode shows its card in the band. */
  onTV: boolean;
  streamUrl: string | null;
  isLoading: boolean;
  mediaType: string | null;
  provider: string;
  userId: string;
}

/** A live stream playing here in the full player. A rewound live stream counts;
 *  a start still loading, a Twitch row with no id yet, an offline room or an
 *  ended stream (`offline`), a cast placeholder and any recording do not. */
export function closeVideoShown(f: CloseVideoFacts): boolean {
  if (f.mini || f.pip || f.onTV || f.isLoading) return false;
  if (!f.streamUrl || f.streamUrl === 'offline' || f.streamUrl.startsWith('cast://')) return false;
  if (f.mediaType !== 'live') return false;
  return f.provider !== 'twitch' || f.userId.trim() !== '';
}

/** Whether a chat tab is this stream's room: the same platform and the same
 *  name, letter case aside. */
export function tabIsStream(
  tab: { provider: string; login: string },
  stream: Pick<TwitchStream, 'user_login' | 'provider'> | null | undefined,
): boolean {
  if (!stream?.user_login) return false;
  return tab.provider === streamProvider(stream) && tab.login.toLowerCase() === stream.user_login.toLowerCase();
}

/** The mini player's close keeps the room when Chats is in front showing the
 *  stream's own room. */
export function miniCloseKeepsChat(s: {
  onChats: boolean;
  activeChannel: string | null;
  streamTab: { channel: string } | null;
}): boolean {
  return s.onChats && !!s.streamTab && s.activeChannel === s.streamTab.channel;
}

export interface CloseVideoDeps {
  /** What plays now (provider and login), or null when nothing does. */
  playing: () => string | null;
  /** The stream's own tab as the mirror has it, or null. */
  streamTab: () => { channel: string; kept: boolean } | null;
  /** Keeps the room; true once Rust's list shows it kept. */
  keep: (channel: string) => Promise<boolean>;
  /** Lets go of a room kept for a stream that is no longer the one playing. */
  forget: (channel: string) => void;
  toChats: () => void;
  close: () => Promise<void>;
  failed: () => void;
}

/** The order: keep the room, then (the same stream still playing) Chats and the
 *  close. A stream switched meanwhile is left playing and a room that was not
 *  yours before is let go; a close by another route still lands on Chats. */
export async function runCloseVideo(d: CloseVideoDeps, toChats: boolean): Promise<void> {
  const started = d.playing();
  if (!started) return;
  const tab = d.streamTab();
  if (!tab) {
    d.failed();
    return;
  }
  const wasKept = tab.kept;
  if (!(await d.keep(tab.channel))) {
    d.failed();
    return;
  }
  const now = d.playing();
  if (now !== null && now !== started) {
    if (!wasKept) d.forget(tab.channel);
    return;
  }
  if (toChats) d.toChats();
  if (now === null) return;
  await d.close();
}

/** One close at a time: a second press while one runs gets the same promise. */
export function closeVideoRunner(make: () => CloseVideoDeps): (toChats: boolean) => Promise<void> {
  let running: Promise<void> | null = null;
  return (toChats) => {
    if (running) return running;
    running = runCloseVideo(make(), toChats).finally(() => {
      running = null;
    });
    return running;
  };
}
