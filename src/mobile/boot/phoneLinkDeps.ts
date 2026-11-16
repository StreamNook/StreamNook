// The phone shell's link pieces, in a plain module with no React so the tests
// drive exactly what useMobileBoot passes: the deps of the one link opener,
// the rule for holding a link, the listeners that release it, and the handlers
// the activity hands warm notification taps to.
import { useMobileNavStore } from '../navStore';
import {
  consumePendingChannelIf,
  consumePendingMentionIf,
  consumePendingWhisperIf,
  isInPip,
  isPipClosing,
} from '../nativeBridge';
import type { InboundLinkDeps, PhoneLinkDoors } from '../../utils/inboundLinkOpen';

/** The phone's deps for `openInboundLink`. */
export function makePhoneLinkDeps({ openChannel }: { openChannel: (key: string) => void }): InboundLinkDeps {
  return {
    startStream: openChannel,
    // A watch link closes every sheet and layer that would sit over the stream.
    closeLinkSheet: () => useMobileNavStore.getState().closeLinkSheet(),
    // The door hands a set and a version; the store keeps one key.
    openBadge: (setId, version) => useMobileNavStore.getState().openBadgeLink(`${setId}/${version}`),
    // Nothing of the shell's own: the badge detail is a sheet on the stack, and
    // openCategoryLink dismisses every sheet itself.
    closeAppLayers: () => {},
    openCategory: (id) => useMobileNavStore.getState().openCategoryLink(id),
  };
}

/** A badge or category link waits while the app is in picture in picture or not visible. */
export function phoneShouldHold(): boolean {
  return isInPip() === true || document.visibilityState !== 'visible';
}

/**
 * A held link opens when the app is back: on every visibility change, and on
 * leaving picture in picture unless the window was dismissed (then it waits
 * for the next visible moment, at most a minute). `sn:pip-closed` alone never
 * releases it. Returns the cleanup.
 */
export function installHeldLinkRelease(release: () => void): () => void {
  const onVisibility = () => release();
  const onPip = () => {
    if (!isPipClosing()) release();
  };
  document.addEventListener('visibilitychange', onVisibility);
  window.addEventListener('sn:pip', onPip);
  return () => {
    document.removeEventListener('visibilitychange', onVisibility);
    window.removeEventListener('sn:pip', onPip);
  };
}

type WarmTapWindow = Window & {
  __SN_OPEN_CHANNEL__?: (login: string, at?: number) => void;
  __SN_OPEN_MENTION__?: (id: string, at?: number) => void;
  __SN_OPEN_WHISPER__?: (key: string, at?: number) => void;
};

/**
 * The handlers the activity hands a warm notification tap to, with the moment
 * it stashed it. Each takes its own stash first, and only while it is still the
 * one stashed at that moment: a tap the mount already opened, or a hand-over
 * that lost a race with a newer tap, opens nothing. Whether the door then opens
 * is the door's rule. Returns the cleanup.
 */
export function installWarmTaps(doors: Pick<PhoneLinkDoors, 'channelTap' | 'mentionTap' | 'whisperTap'>): () => void {
  const w = window as WarmTapWindow;
  w.__SN_OPEN_CHANNEL__ = (login, at = 0) => {
    if (!login) return;
    if (!consumePendingChannelIf(at)) return;
    doors.channelTap(login, at);
  };
  w.__SN_OPEN_MENTION__ = (id, at = 0) => {
    if (!id) return;
    if (!consumePendingMentionIf(at)) return;
    doors.mentionTap(id, at);
  };
  w.__SN_OPEN_WHISPER__ = (key, at = 0) => {
    if (!key) return;
    if (!consumePendingWhisperIf(at)) return;
    doors.whisperTap(key, at);
  };
  return () => {
    delete w.__SN_OPEN_CHANNEL__;
    delete w.__SN_OPEN_MENTION__;
    delete w.__SN_OPEN_WHISPER__;
  };
}
