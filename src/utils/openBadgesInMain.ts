// Route profile-card surface opens (badge overlays + the public profile viewer)
// to the main window.
//
// Every Tauri WebView has its own JS context and its own AppStore instance,
// so calling `useAppStore.getState().openBadgesWithBadge(...)` inside a
// MultiChat popout flips state on the popout's own store — the overlay
// opens in the popout, not in the main app. That's not what we want for a
// chat-only popout: the badges/paints picker is a main-app surface and the
// popout is meant to stay slim. These helpers detect whether we're in a
// popout and either dispatch via a Tauri event (popout → main) or fall
// through to the local store (main → main).
//
// Main-window listeners live in `utils/multichatTrayBridge.ts` next to the
// other popout-to-main bridges, so anyone tracing the event flow can find
// every listener in one place.
//
// Caller contract: callers don't need to know whether they're in a popout.
// They just call the helper and the right thing happens.

import { useAppStore } from '../stores/AppStore';
import { Logger } from './logger';

function isPopoutWindow(): boolean {
  if (typeof window === 'undefined') return false;
  const hash = window.location.hash;
  return hash.startsWith('#/multichat') || hash.startsWith('#/profile');
}

async function emitToMain(eventName: string, payload?: unknown): Promise<void> {
  try {
    // Going live may have closed the main window; ensure it's back + listening
    // before emitting (a fast show+focus if it was only hidden/already open).
    const { ensureMainAndEmit } = await import('./ensureMainWindow');
    Logger.debug(`[openBadgesInMain] emitting ${eventName}`, payload);
    await ensureMainAndEmit(eventName, payload ?? {});
    Logger.debug(`[openBadgesInMain] emit ${eventName} done`);
  } catch (err) {
    Logger.error(`[openBadgesInMain] emit ${eventName} failed:`, err);
  }
}

export function openBadgesWithBadgeInMain(badgeId: string): void {
  const popout = isPopoutWindow();
  Logger.debug(`[openBadgesInMain] openBadgesWithBadge popout=${popout} badgeId=${badgeId}`);
  if (popout) {
    void emitToMain('open-badges-with-badge', { badgeId });
    return;
  }
  useAppStore.getState().openBadgesWithBadge(badgeId);
}

export function openBadgesWithPaintInMain(paintId: string): void {
  const popout = isPopoutWindow();
  Logger.debug(`[openBadgesInMain] openBadgesWithPaint popout=${popout} paintId=${paintId}`);
  if (popout) {
    void emitToMain('open-badges-with-paint', { paintId });
    return;
  }
  useAppStore.getState().openBadgesWithPaint(paintId);
}

export function openBadgesOnStreamNookInMain(): void {
  const popout = isPopoutWindow();
  Logger.debug(`[openBadgesInMain] openBadgesOnStreamNook popout=${popout}`);
  if (popout) {
    void emitToMain('open-badges-on-streamnook');
    return;
  }
  useAppStore.getState().openBadgesOnStreamNook();
}

// Generic deep-link: open the overlay on a given tab and (optionally) filter to
// a badge title. Used for badge types without a dedicated detail modal (Twitch,
// BetterTTV, Chat Clients) clicked from the profile card.
export function openBadgesWithTargetInMain(target: { tab: string; query?: string }): void {
  const popout = isPopoutWindow();
  Logger.debug(`[openBadgesInMain] openBadgesWithTarget popout=${popout}`, target);
  if (popout) {
    void emitToMain('open-badges-with-target', target);
    return;
  }
  useAppStore.getState().openBadgesWithTarget(target);
}


// Open a specific badge's detail in the badges overlay — clicked on a chat MESSAGE
// (Twitch/BTTV/etc.). The overlay lives ONLY in main, so from a popout we ensure
// main is open + emit; in main we drive the store + detail event directly. Without
// this, a message-badge click in a popout flipped the popout's own (overlay-less)
// store and did nothing — only 7TV cosmetics, which have their own main-routing
// handlers, opened main.
export function openBadgeDetailInMain(badge: unknown, setId: string): void {
  const popout = isPopoutWindow();
  Logger.debug(`[openBadgesInMain] openBadgeDetail popout=${popout} setId=${setId}`);
  if (popout) {
    void emitToMain('open-badge-detail', { badge, setId });
    return;
  }
  useAppStore.getState().setShowBadgesOverlay(true);
  window.dispatchEvent(new CustomEvent('show-badge-detail', { detail: { badge, setId } }));
}
