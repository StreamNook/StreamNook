// Mobile shell boot orchestration.
//
// DELIBERATE DUPLICATION, KEEP IN SYNC: the desktop boot lives inline in
// src/App.tsx (the big effect starting near line 537). We replicate the
// SHELL-AGNOSTIC calls here rather than extracting a shared hook, because the
// desktop closure interleaves them with desktop-only listeners and splitting it
// is the riskiest refactor in the repo. When a dev merge adds a new boot step
// to App.tsx, decide whether it belongs here too (see the cross-reference
// comment in App.tsx).
//
// Deliberately NOT replicated (desktop-only surfaces): session resume, whisper
// import, eventsub://channel-moderate (mod tools), drop-progress automation,
// snippet sync, window sizing, universal-cache disk sync (features.assetDiskCache).
import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import {
  handleSeventvCosmeticUpdate,
  handleSeventvEmoteSetUpdate,
  type CosmeticUpdatePayload,
  type EmoteSetUpdatePayload,
} from '../../services/seventvEventApi';
import { useWatchedChestStat } from '../../hooks/useWatchedChestStat';
import {
  isSupabaseConfigured,
  refreshEntitlementRegistries,
  subscribeToAtmospheresRegistry,
  subscribeToCosmeticsRegistry,
  subscribeToStreamNookRegistry,
  trackPresence,
} from '../../services/supabaseService';
import {
  NOTIFY_CHANNEL,
  ensureNotificationChannels,
  ensureNotificationPermission,
  getNotificationPermission,
  postSystemNotification,
  stableNotifyId,
  syncBackgroundChecks,
} from '../notifications';
import { consumePendingChannel } from '../nativeBridge';
import { applyPlaybackAdapt, type PlaybackAdapt } from '../player/linkAdapt';
import { useFollowsStore, type ProviderStreamRow } from '../../stores/followsStore';
import type { ProviderId } from '../../types/providers';
import { streamProvider } from '../../utils/streamProvider';
import { Logger } from '../../utils/logger';
import { SENDER_ID, SETTINGS_UPDATED_EVENT, type SettingsUpdatedPayload } from '../../utils/settingsBroadcast';

export function useMobileBoot(): void {
  // Lifetime points stat: the same hook desktop mounts in App.tsx, counting
  // `watched-chest-claimed`. Every chest, automatic or a tap on the composer's
  // button, goes through Rust's watched_chest::claim, which emits that event
  // once per chest. The phone used to count `channel-points-earned` instead,
  // which other sources also emit (the points socket, the balance poll), so
  // the two shells scored different things. Mounted once, here, because the
  // phone has a single window.
  useWatchedChestStat();

  useEffect(() => {
    let isMounted = true;
    const cleanupFunctions: (() => void)[] = [];

    const addListener = async <T,>(event: string, handler: (event: { payload: T }) => void) => {
      try {
        const unlistenFn = await listen<T>(event, handler);
        if (isMounted) cleanupFunctions.push(unlistenFn);
        else unlistenFn();
      } catch (e) {
        Logger.warn(`[MobileBoot] Failed to set up listener for ${event}:`, e);
      }
    };

    // Same unmount discipline for everything that is not a Tauri listener.
    // initialize() awaits between registrations, so a cleanup pushed after the
    // effect tore down would land in an array nobody iterates again; running it
    // immediately instead is what stops a dead effect leaking timers and
    // subscriptions.
    const addCleanup = (fn: () => void) => {
      if (isMounted) cleanupFunctions.push(fn);
      else fn();
    };

    // False until a settings read has actually succeeded. A failed read must
    // not stop the boot (every listener below would go missing with it), but
    // it must not be mistaken for the user's choices either: the store then
    // holds an EMPTY settings object, never defaults, and the steps below that
    // act on a preference (the background-check schedule, the push register)
    // wait for a real read rather than treating "absent" as "on". Nothing here
    // saves settings, and the store's saves send Rust only the top-level keys
    // a change touched, so an empty store never writes a whole defaults file.
    let settingsLoaded = false;

    const initialize = async () => {
      const store = useAppStore.getState();
      try {
        try {
          await store.loadSettings();
          settingsLoaded = true;
        } catch (err) {
          // One retry: the store's own comment says an early read can race
          // Rust's state management, and that race clears in moments.
          Logger.warn('[MobileBoot] settings load failed, retrying:', err);
          await new Promise((r) => setTimeout(r, 750));
          try {
            await useAppStore.getState().loadSettings();
            settingsLoaded = true;
          } catch (retryErr) {
            Logger.error('[MobileBoot] settings load failed; booting without them:', retryErr);
          }
        }
        await store.checkAuthStatus();
      } catch (err) {
        Logger.error('[MobileBoot] auth check failed during boot:', err);
      } finally {
        // Auth resolved (logged in or confirmed logged out) or a boot step
        // failed; either way drop the boot overlay.
        if (isMounted) useAppStore.setState({ isBooting: false });
      }

      // Rust announces every settings write: a list edit, a field write, a
      // reset. Re-read so the phone's screens follow, as desktop's App.tsx
      // does. Without it a write made in Rust (the live-alert bell, the shared
      // list editors) left the store, and every screen reading it, stale.
      void addListener<SettingsUpdatedPayload>(SETTINGS_UPDATED_EVENT, (event) => {
        if (event.payload?.source === SENDER_ID) return;
        useAppStore
          .getState()
          .loadSettings()
          .catch((err) => Logger.warn('[MobileBoot] settings re-read failed:', err));
      });

      // Is the drops token actually still good?
      //
      // Every other surface asks `is_drops_authenticated`, which only checks
      // that a token file exists, never that Twitch still accepts it. The
      // device-code client has no secret, so it cannot refresh: once a token
      // dies (password change, session revoke) it stays dead. Nothing noticed,
      // because the failure is silent all the way down - the watch heartbeat
      // just returns early and earns nothing, the drops panel keeps saying
      // connected, and the only trace is a warn line in logcat.
      //
      // `validate_drops_token` deletes the stored token and cookies on a 401,
      // so one call at boot converts that silent permanent failure into an
      // honest "not connected" the user can act on. Fire and forget: it is a
      // single request, nothing downstream waits on it, and being offline at
      // boot must not be mistaken for a bad token (the command only clears on
      // an actual 401, not on a transport error).
      void invoke<boolean>('validate_drops_token')
        .then((ok) => {
          if (!ok) Logger.warn('[MobileBoot] drops token rejected; cleared, reconnect needed');
        })
        .catch((err) => {
          Logger.debug('[MobileBoot] drops token validation skipped:', err);
        });

      // Active drops cache (1h TTL); powers the Activity surface.
      useAppStore.getState().loadActiveDropsCache();

      // The real-time badge-drop feed is owned by Rust (services::badge_feed,
      // started in lib.rs setup) and pushes events; nothing to start here.

      // Pre-fetch cosmetics for the signed-in account(s) so chat and profile
      // paint on frame one. Mirrors the desktop block in App.tsx.
      const { currentUser, isAuthenticated } = useAppStore.getState();
      if (isAuthenticated && currentUser?.user_id) {
        const { registerOwnCosmeticAccounts, revalidateOwnCosmetics, getFullProfileWithFallback } =
          await import('../../services/cosmeticsCache');
        const { seedOwnIdentitiesFromCache, getResolvedIdentity, getIdentityWithCache } =
          await import('../../services/identityService');
        const { registerOwnAtmospheres } = await import('../../stores/chatUserStore');
        const { listAccounts } = await import('../../services/accountService');
        const selfId = currentUser.user_id;
        const selfLogin = currentUser.login || currentUser.username;

        // Keep the 7TV session alive. Its tokens last 30 days and an expired one
        // reads as signed out; a hidden reload of the sign-in re-mints it while
        // the Twitch session is still good. Deferred past the boot burst and
        // throttled inside, so it costs nothing on most launches.
        setTimeout(() => {
          void Promise.all([
            import('../../services/sevenTvSession'),
            import('../cosmetics/sevenTvConnect'),
          ]).then(([session, connect]) =>
            session.maybeRefreshSevenTvSession(() => connect.refreshSevenTvSilently(selfId)),
          );
        }, 8000);

        let accountIds = [selfId];
        try {
          const ids = (await listAccounts()).map((a) => a.user_id).filter(Boolean);
          if (ids.length) accountIds = ids.includes(selfId) ? ids : [...ids, selfId];
        } catch {
          /* account registry not ready yet — fall back to the active account */
        }

        registerOwnCosmeticAccounts(accountIds);
        seedOwnIdentitiesFromCache(accountIds);
        registerOwnAtmospheres(accountIds);

        revalidateOwnCosmetics(selfId)
          .then(() => getFullProfileWithFallback(selfId, selfLogin, selfId, selfLogin))
          .catch((err: Error) =>
            Logger.error('[MobileBoot] Failed to pre-fetch user profile:', err),
          );
        getResolvedIdentity(selfId).catch(() => {});
        getIdentityWithCache(selfId).catch(() => {});
        for (const id of accountIds) {
          if (id === selfId) continue;
          revalidateOwnCosmetics(id).catch(() => {});
          getResolvedIdentity(id).catch(() => {});
          getIdentityWithCache(id).catch(() => {});
        }
      }

      // Live 7TV emote-set updates + cosmetics pushed from the Rust EventAPI socket.
      await addListener<EmoteSetUpdatePayload>('7tv://emote-set-update', (event) => {
        void handleSeventvEmoteSetUpdate(event.payload);
      });
      await addListener<CosmeticUpdatePayload>('7tv://cosmetic-update', (event) => {
        void handleSeventvCosmeticUpdate(event.payload);
      });

      // Ad auto-pivot: backend re-resolved a clean player URL; applying it
      // changes streamUrl, which the mobile hls engine reloads from.
      await addListener<{ url: string; region?: string; channel?: string }>('ad-pivot', (event) => {
        const { url, region } = event.payload;
        if (url) useAppStore.getState().applyAdPivot(url, region);
      });

      // The core's link governor moved the rendition or the cushion (a phone
      // on a poor link). A URL means the relay restarted and the player
      // reloads on it; otherwise the engine picks the cushion up by itself.
      await addListener<PlaybackAdapt>('playback-adapt', (event) => {
        applyPlaybackAdapt(event.payload);
      });

      // Follow/unfollow actions elsewhere ask the shell to refresh the list.
      await addListener<void>('refresh-following-list', () => {
        void useAppStore.getState().loadFollowedStreams();
      });

      // Kick follows. The app keeps its own list (Kick exposes none to us) and
      // Rust's provider_live_service owns liveness, pushing provider-live-update.
      // The initial pull only paints what it already knows. Mirrors App.tsx.
      void useFollowsStore.getState().hydrate();
      void useFollowsStore.getState().refreshLive();
      await addListener<{ provider: ProviderId; streams: ProviderStreamRow[] }>(
        'provider-live-update',
        (event) => {
          useFollowsStore.getState().setProviderLive(event.payload.provider, event.payload.streams ?? []);
        },
      );
      // The Kick stream being watched ended, reported the moment the backend
      // learns it. Without this the player froze on its last frame until the
      // slower liveness poll agreed. Same match as desktop.
      await addListener<{ provider: ProviderId; channel: string }>('provider-stream-offline', (event) => {
        const watching = useAppStore.getState().currentStream;
        if (!watching) return;
        const sameChannel =
          (watching.user_login || '').toLowerCase() === (event.payload.channel || '').toLowerCase();
        if (streamProvider(watching) !== event.payload.provider || !sameChannel) return;
        void useAppStore.getState().handleStreamOffline();
      });

      // ---- System notifications -------------------------------------------
      //
      // Every one of these listens to the event RUST emits, never to a
      // re-broadcast. That distinction is the whole reason none of them worked.
      //
      // Live alerts were wired to `show-live-toast`, which sounds like the
      // backend's event and is not: Rust emits `streamer-went-live`, and the
      // ONLY thing that turns it into `show-live-toast` is `DynamicIsland.tsx`,
      // a desktop component the phone never renders. So the backend was firing
      // correctly the whole time into a relay that does not exist here. The
      // other three categories had no listener at all, which is why the panel
      // offered five toggles and exactly none of them did anything.
      //
      // The Rust services behind these all start unconditionally, so nothing
      // needed enabling on that side.
      const notifyPrefs = () => useAppStore.getState().settings.live_notifications;

      // Register the OS categories before anything can post. Idempotent, and
      // what gives Android's own per-app notification settings a row per
      // category instead of one switch for the whole app.
      void ensureNotificationChannels();

      // Ask for the notification permission once, for people the wizard cannot
      // reach. Android 13+ grants nothing by default, and the wizard step only
      // ever runs on a first-time setup, so anyone who updated in place has
      // never been asked and has no way of knowing that is why they hear
      // nothing. Only when setup is already complete (a fresh install gets the
      // wizard's own prompt) and only when Android will still show the dialog:
      // once it has decided the refusal is permanent, asking again does nothing
      // and the settings panel is the only honest route.
      void (async () => {
        if (!useAppStore.getState().settings.setup_complete) return;
        if ((await getNotificationPermission()) !== 'default') return;
        await ensureNotificationPermission();
      })();

      // Keep the background poll in step with the settings on every launch, so
      // scheduled work can never outlive the preference that asked for it.
      // Skipped when settings never loaded: an empty store reads as "on", and
      // the schedule from the last good launch is the honest one to keep.
      if (settingsLoaded) syncBackgroundChecks(notifyPrefs());

      // Instant push: upload the FCM token + follow list to the notify server.
      // Fire and forget; "no token yet" and network failures are both normal
      // (the poll lane delivers regardless), and re-running every launch is
      // what keeps the server's follow list current. Same guard as above: a
      // user who turned push off must not be registered by a failed read.
      if (settingsLoaded) {
        const prefs = notifyPrefs();
        if (prefs?.enabled !== false && prefs?.push_notifications !== false) {
          void invoke<string>('push_register')
            .then((r) => Logger.debug('[MobileBoot] push:', r))
            .catch((err) => Logger.debug('[MobileBoot] push register skipped:', err));
        }
      }

      // Open a channel handed over from outside the app: a tapped notification,
      // or a streamnook:// link.
      //
      // A deep link carries a startStream key (bare Twitch login or
      // `provider:channel`); its YouTube video id is case-sensitive, so the
      // key is passed on as given and startStream folds Twitch case itself.
      // Rust stashes every phone link before it emits `streamnook:watch`, so
      // the event and the mount both drain `take_pending_watch_link`: whichever
      // takes it opens it, once. The listener goes up before the mount drain so
      // a link landing in between is not missed.
      const openChannel = (login: string) => {
        const clean = login.trim();
        if (!clean) return;
        void useAppStore.getState().startStream(clean);
      };
      const drainWatchLink = () =>
        invoke<string | null>('take_pending_watch_link')
          .then((link) => {
            if (link) openChannel(link);
          })
          .catch(() => {});
      (window as Window & { __SN_OPEN_CHANNEL__?: (login: string) => void }).__SN_OPEN_CHANNEL__ =
        openChannel;
      await addListener<string>('streamnook:watch', () => {
        void drainWatchLink();
      });
      const tapped = consumePendingChannel();
      if (tapped) {
        openChannel(tapped);
      } else {
        void drainWatchLink();
      }

      // No `streamer-went-live` listener here anymore, on purpose. The
      // WorkManager poll (android_notify.rs) is the ONLY live-alert lane, app
      // open or closed: it has the persistent started_at dedupe and the stable
      // notification ids this listener never had, and the Rust 60s poll that
      // fed this event no longer runs on Android at all.

      // Badges arrive as an ARRAY, since the scanner can surface several at
      // once. One notification each; there are rarely more than a couple. The
      // id is the same key the background worker hashes, so however many lanes
      // see a badge, the shade holds one row for it.
      await addListener<
        Array<{
          badge_set_id?: string;
          badge_version?: string;
          badge_name: string;
          badge_image_url?: string;
          badge_description?: string;
        }>
      >('badge-notification', (event) => {
        const prefs = notifyPrefs();
        if (prefs?.enabled === false || prefs?.show_badge_notifications === false) return;
        for (const badge of event.payload ?? []) {
          void postSystemNotification({
            id: stableNotifyId(`badge:${badge.badge_set_id}-v${badge.badge_version}`),
            title: `New badge: ${badge.badge_name}`,
            body: badge.badge_description,
            channelId: NOTIFY_CHANNEL.badges,
          });
        }
      });

      await addListener<{ drop_name?: string; campaign_name?: string }>('drop-ready', (event) => {
        const prefs = notifyPrefs();
        if (prefs?.enabled === false || prefs?.show_drops_notifications === false) return;
        const d = event.payload;
        void postSystemNotification({
          title: 'Drop ready to claim',
          body: [d?.drop_name, d?.campaign_name].filter(Boolean).join(' · ') || undefined,
          channelId: NOTIFY_CHANNEL.drops,
        });
      });

      await addListener<{ benefit_name?: string; campaign_name?: string }>(
        'drop-claimed',
        (event) => {
          const prefs = notifyPrefs();
          if (prefs?.enabled === false || prefs?.show_drops_notifications === false) return;
          const d = event.payload;
          void postSystemNotification({
            title: 'Drop claimed',
            body: [d?.benefit_name, d?.campaign_name].filter(Boolean).join(' · ') || undefined,
            channelId: NOTIFY_CHANNEL.drops,
          });
        },
      );

      // Supabase presence + the server-driven registries (membership, cosmetics
      // catalog + ownership, atmospheres). Without these the cosmetics surfaces
      // read empty: getOwnedCosmeticSlugs and friends serve these in-memory
      // registries. Re-pull on return to the app, throttled, using
      // visibilitychange (mobile has no meaningful window focus).
      if (isSupabaseConfigured()) {
        const { currentUser: presenceUser, isAuthenticated: presenceAuthed } =
          useAppStore.getState();
        try {
          const cleanupPresence = await trackPresence(
            presenceAuthed ? presenceUser?.user_id : undefined,
            presenceAuthed ? presenceUser?.display_name : undefined,
            undefined,
          );
          if (cleanupPresence) addCleanup(cleanupPresence);
        } catch (e) {
          Logger.warn('[MobileBoot] presence failed:', e);
        }

        const cleanupRegistry = subscribeToStreamNookRegistry();
        const cleanupCosmetics = subscribeToCosmeticsRegistry();
        const cleanupAtmospheres = subscribeToAtmospheresRegistry();
        addCleanup(() => {
          cleanupRegistry?.();
          cleanupCosmetics?.();
          cleanupAtmospheres?.();
        });

        let lastResync = 0;
        const onVisible = () => {
          if (document.visibilityState !== 'visible') return;
          const now = Date.now();
          if (now - lastResync < 10_000) return;
          lastResync = now;
          refreshEntitlementRegistries();
        };
        document.addEventListener('visibilitychange', onVisible);
        addCleanup(() =>
          document.removeEventListener('visibilitychange', onVisible),
        );
      }
    };
    void initialize();

    // Periodic auth recheck: keeps tokens fresh across long sessions. Skipped
    // while hidden; a backgrounded shell has nothing to do with a fresh token,
    // and the next visible tick (or the boot check on relaunch) covers it.
    const authInterval = setInterval(() => {
      if (document.visibilityState !== 'visible') return;
      void useAppStore.getState().checkAuthStatus();
    }, 5 * 60 * 1000);
    cleanupFunctions.push(() => clearInterval(authInterval));

    return () => {
      isMounted = false;
      for (const fn of cleanupFunctions) fn();
    };
    // Boot runs once; everything is read through getState().
  }, []);
}
