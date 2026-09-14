// Main-window-only bridge to two Rust-side events:
//
//  - `tray-open-multichat`: tray menu "Open MultiChat" → spawn an empty popout.
//  - `main-hiding-to-tray`: main window's close button was intercepted because
//    popouts are open. Before the window actually hides we shut down the
//    stream so the user isn't paying for Streamlink + video decoding + audio
//    while they're in "chat-only" mode. Chat stays alive because the popouts
//    need it — we deliberately do NOT call `stop_chat` here.
//
// Both listeners gate on URL hash so the popout windows ignore these events
// (the popout window's own JS context shouldn't react to tray events).

import { invoke } from '@tauri-apps/api/core';
import type { MediaInfo } from '../stores/AppStore';
import { Logger } from './logger';

const hash = window.location.hash;
const isPopout = hash.startsWith('#/multichat') || hash.startsWith('#/profile');

/** The core app's X button was intercepted (popouts open, or the "Always
 *  minimize" close mode). Stop everything that costs CPU/network, THEN destroy the
 *  main window to actually free its memory — same outcome as Go Live. Chat (IRC)
 *  is intentionally left alive because popouts may be subscribed to it; main
 *  recreates on demand from the tray.
 *
 *  Under "Always minimize" with no popouts this destroy leaves the process with
 *  zero windows. Rust's ExitRequested handler in main.rs keeps it alive in that
 *  mode; without it Tauri exits on the last window and the tray icon dies too. */
async function stopStreamButKeepChat(): Promise<void> {
  // Lazy import to avoid pulling AppStore into the popout bundle's chunking.
  const { useAppStore } = await import('../stores/AppStore');
  const { currentStream } = useAppStore.getState();

  // Clear the in-memory stream state so the main window's UI returns to a
  // clean home screen next time the user opens it via the tray. Done BEFORE
  // stop_stream so the player unmounts (VideoPlayer is keyed on streamUrl)
  // and isn't left polling the dead relay port.
  useAppStore.setState({
    streamUrl: null,
    activeQuality: null,
    currentStream: null,
    currentMediaType: null,
  });

  try {
    await invoke('stop_stream');
  } catch (err) {
    Logger.warn('[TrayBridge] stop_stream failed:', err);
  }

  try {
    await invoke('stop_drops_monitoring');
  } catch (err) {
    Logger.warn('[TrayBridge] stop_drops_monitoring failed:', err);
  }

  if (currentStream?.user_id) {
    invoke('unregister_active_channel', { channelId: currentStream.user_id }).catch(() => {});
  }

  // Destroy main to actually free its memory (the ~350MB webview + player), same as
  // Go Live — the popouts keep the app alive and main recreates on demand. Rust
  // already hid it for an instant disappear; this frees it. Fire-and-forget: this
  // window is going away, so the invoke may not resolve.
  void invoke('close_main_window');
  Logger.debug('[TrayBridge] Stream stopped + main closed; chat preserved in popouts');
}

// Per-popout-window channel ownership — windowLabel → set of channel logins.
// Each popout emits its full channel set on every change; we replace this
// window's entry and recompute the aggregate so the main app can know "is
// channel X open in any popout?" in O(1).
const popoutChannelMap = new Map<string, Set<string>>();

async function pushAggregateToStore() {
  const aggregate = new Set<string>();
  for (const set of popoutChannelMap.values()) {
    for (const ch of set) aggregate.add(ch);
  }
  const { useAppStore } = await import('../stores/AppStore');
  // Always replace the Set reference so Zustand consumers re-render — the
  // store's contract treats this as immutable state.
  useAppStore.setState({ channelsInPopouts: aggregate });
}

if (!isPopout && typeof window !== 'undefined') {
  (async () => {
    try {
      const { listen, emit } = await import('@tauri-apps/api/event');

      await listen('tray-open-multichat', async () => {
        Logger.debug('[TrayBridge] tray-open-multichat received, spawning empty popout');
        try {
          const { openMultiChatWindow } = await import('./multichatWindow');
          await openMultiChatWindow({});
        } catch (err) {
          Logger.error('[TrayBridge] openMultiChatWindow failed:', err);
        }
      });

      await listen('main-hiding-to-tray', () => {
        Logger.debug('[TrayBridge] main-hiding-to-tray received, soft-stopping stream');
        void stopStreamButKeepChat();
      });

      // Popout windows emit this on every channel-list change. Replace this
      // window's entry and push the new aggregate into AppStore so the
      // main-app chat panel knows which channels are owned by popouts.
      await listen<{ windowLabel: string; channels: string[] }>(
        'multichat-popout-channels',
        (event) => {
          const { windowLabel, channels } = event.payload;
          if (channels.length === 0) {
            popoutChannelMap.delete(windowLabel);
          } else {
            popoutChannelMap.set(
              windowLabel,
              new Set(channels.map((c) => c.toLowerCase())),
            );
          }
          void pushAggregateToStore();
        },
      );

      // Rust emits this when a multichat-* window is Destroyed. Drop its
      // tracking entry so its channels stop counting against main's hide
      // logic; main's ChatWidget restores for any channel that was only in
      // that popout.
      await listen<string>('multichat-popout-closed', (event) => {
        const windowLabel = event.payload;
        popoutChannelMap.delete(windowLabel);
        void pushAggregateToStore();
      });

      // Badge / paint / StreamNook-tab overlay opens, routed from popouts.
      // Each popout has its own AppStore, so a badge click there would open
      // the overlay in the popout itself — we want it in main. The popout
      // emits one of these events via `utils/openBadgesInMain.ts`; main
      // un-hides (if tray-hidden), focuses, then dispatches to its own store.
      const showAndFocusMain = async () => {
        try {
          const { getCurrentWindow } = await import('@tauri-apps/api/window');
          const win = getCurrentWindow();
          await win.unminimize().catch(() => {});
          await win.show().catch(() => {});
          await win.setFocus().catch(() => {});
        } catch (err) {
          Logger.warn('[TrayBridge] show main failed:', err);
        }
      };

      await listen<{ badgeId: string }>('open-badges-with-badge', async (event) => {
        Logger.debug('[TrayBridge] open-badges-with-badge received', event.payload);
        const { badgeId } = event.payload;
        await showAndFocusMain();
        const { useAppStore } = await import('../stores/AppStore');
        useAppStore.getState().openBadgesWithBadge(badgeId);
      });

      await listen<{ paintId: string }>('open-badges-with-paint', async (event) => {
        Logger.debug('[TrayBridge] open-badges-with-paint received', event.payload);
        const { paintId } = event.payload;
        await showAndFocusMain();
        const { useAppStore } = await import('../stores/AppStore');
        useAppStore.getState().openBadgesWithPaint(paintId);
      });

      await listen('open-badges-on-streamnook', async () => {
        Logger.debug('[TrayBridge] open-badges-on-streamnook received');
        await showAndFocusMain();
        const { useAppStore } = await import('../stores/AppStore');
        useAppStore.getState().openBadgesOnStreamNook();
      });

      // A badge clicked on a chat message in a popout (Twitch/BTTV/etc.). The badges
      // overlay lives only here in main, so show main + open the badge's detail.
      await listen<{ badge: unknown; setId: string }>('open-badge-detail', async (event) => {
        Logger.debug('[TrayBridge] open-badge-detail received', event.payload);
        await showAndFocusMain();
        const { useAppStore } = await import('../stores/AppStore');
        useAppStore.getState().setShowBadgesOverlay(true);
        window.dispatchEvent(
          new CustomEvent('show-badge-detail', {
            detail: { badge: event.payload.badge, setId: event.payload.setId },
          }),
        );
      });

      await listen<{ tab: string; query?: string }>('open-badges-with-target', async (event) => {
        Logger.debug('[TrayBridge] open-badges-with-target received', event.payload);
        await showAndFocusMain();
        const { useAppStore } = await import('../stores/AppStore');
        useAppStore.getState().openBadgesWithTarget(event.payload);
      });

      // Public StreamNook profile viewer, routed from a profile-card popout (or
      // a MultiChat popout): PublicProfileOverlay lives only in main, so the
      // popout emits this and main shows + opens it. The popout closes itself
      // (see openProfileViewerInMain) so its alwaysOnTop window can't cover it.
      await listen<{ userId: string }>('open-profile-viewer', async (event) => {
        Logger.debug('[TrayBridge] open-profile-viewer received', event.payload);
        const { userId } = event.payload;
        await showAndFocusMain();
        const { useAppStore } = await import('../stores/AppStore');
        useAppStore.getState().openProfileViewer(userId);
      });

      // Restore-to-main flow: popout asks main to show itself + start watching
      // a channel. Used by:
      //   - Restore-to-main button (single-tab popouts) — popout closes itself
      //     after firing.
      //   - Tab right-click → "Watch in main app" — popout removes just that
      //     tab and keeps running.
      // Either way main is responsible for un-hiding (if tray-hidden) and
      // calling startStream. The popout's auto-hide-main behavior takes over
      // immediately after if it still owns the channel, so this flow is
      // explicitly "no, main should own this channel from now on."
      await listen<{ channel: string; channelId?: string; channelName?: string }>(
        'watch-channel-in-main',
        async (event) => {
          const { channel } = event.payload;
          Logger.debug(`[TrayBridge] watch-channel-in-main: ${channel}`);
          try {
            const { getCurrentWindow } = await import('@tauri-apps/api/window');
            const win = getCurrentWindow();
            await win.unminimize().catch(() => {});
            await win.show().catch(() => {});
            await win.setFocus().catch(() => {});
          } catch (err) {
            Logger.warn('[TrayBridge] show main failed:', err);
          }
          try {
            const { useAppStore } = await import('../stores/AppStore');
            await useAppStore.getState().startStream(channel);
          } catch (err) {
            Logger.error('[TrayBridge] startStream failed:', err);
          }
        },
      );

      // Twitch clip/VOD playback routed from a popout chat. The popout has no
      // video player, so a clip/VOD link-preview card there emits this (via
      // `utils/playTwitchMediaInMain.ts`); main un-hides, focuses, and plays.
      await listen<{ type: 'clip' | 'video'; url: string; info: MediaInfo }>(
        'play-twitch-media-in-main',
        async (event) => {
          const { type, url, info } = event.payload;
          Logger.debug(`[TrayBridge] play-twitch-media-in-main: ${type} ${url}`);
          await showAndFocusMain();
          const { useAppStore } = await import('../stores/AppStore');
          await useAppStore.getState().playMedia(type, url, info);
        },
      );

      Logger.debug('[TrayBridge] tray listeners registered');
      // Tell any popout that just recreated this window (via ensureMainAlive) that
      // main's popout->main listeners are now live, so it can safely emit its
      // pending action (badge overlay, profile viewer, whisper, watch-in-main,
      // clip/VOD). Fired on every main boot — harmless when nothing is waiting.
      void emit('main-ready');
    } catch (err) {
      Logger.warn('[TrayBridge] Failed to register tray listeners:', err);
    }
  })();
}
