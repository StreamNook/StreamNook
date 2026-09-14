// commandPaletteSources — catalog + dynamic providers feeding the Ctrl+K palette.
//
// Source families:
//   1. **Static catalog**  — quick actions + every Settings tab/section.
//      Captured once at module load; each item carries a `run()` that performs
//      its action against the (window-local) AppStore.
//   2. **Live store readers** — followed channels, recent chatters. Recomputed
//      from `useAppStore.getState()` / `useChatUserStore.getState()` per query
//      so newly-live channels and just-arrived chatters show up without a
//      refresh.
//   3. **Snippets** — bundled Twitch copypastas + slash-command snippets.
//      Selecting copies the snippet body to the clipboard and toasts.
//   4. **Twitch live search** — debounced Helix `search_channels`.
//   5. **Twitch categories** — debounced Helix `search_categories`. Each
//      match expands into "Browse {Game}" + "View drops for {Game}" rows.
//
// All sources flatten to `PaletteItem` so the renderer doesn't need to know
// where a result came from. `section` drives grouping; `score` is filled in by
// the matcher at query time.

import { invoke } from '@tauri-apps/api/core';
import { helixGet } from '../services/helix';
import { useAppStore, clipSourceOf, type SettingsTab } from '../stores/AppStore';
import { useChatUserStore } from '../stores/chatUserStore';
import { useFollowsStore } from '../stores/followsStore';
import { favoriteIdOf, favoriteMetaOf } from './favorites';
import { useSnippetStore } from '../stores/snippetStore';
import { WATCHABLE_PROVIDERS, providerLabel, type ProviderId } from '../types/providers';
import { usePluginUiRegistry } from '../plugins-ui/registry';
import { Logger } from './logger';
import { getBuiltInSnippets, type Snippet } from './commandPaletteCopypastas';
import type { TwitchStream, TwitchVideo, TwitchClip } from '../types';
import type { ChannelAboutData, SocialMediaLink } from '../types/panels';
import { getShortcutDisplayMap } from '../keybindings/registry';
import { clearAllRecentSearches } from './searchHistory';
import { getPlayerControls } from '../keybindings/playerControls';
import { getBindableCommand } from '../keybindings/commands';

// Lazy-import multichatWindow to match the dynamic-import pattern other
// consumers (ChatWidget, StreamContextMenu, commandHandler, multichatTrayBridge)
// already use. A static import here would add a new static-vs-dynamic warning
// to the build output.
async function spawnMultiChat(options: Parameters<typeof import('./multichatWindow').openMultiChatWindow>[0]): Promise<void> {
  const { openMultiChatWindow } = await import('./multichatWindow');
  return openMultiChatWindow(options);
}

export type PaletteSection =
  | 'Recent'
  | 'Now Playing'
  | 'Jump To'
  | 'Tips'
  | 'Quick Actions'
  | 'Current Stream'
  | 'Share'
  | 'Snippets'
  | 'Categories'
  | 'Settings'
  | 'Followed Channels'
  | 'Recent Chatters'
  | 'Streamers';

export interface PaletteItem {
  /** Stable id used for keyboard nav + recent-commands persistence. */
  id: string;
  section: PaletteSection;
  title: string;
  /** Short secondary line (e.g. "Settings · Player", game name + viewer count). */
  subtitle?: string;
  /** Third line — only rendered when the row is active. Used for streamer
   *  bio enrichment so the description is shown only on demand, not for
   *  every visible row. */
  details?: string;
  /** Optional avatar URL (streamer thumbnails). */
  avatarUrl?: string;
  /** Optional fallback letter for items without avatars. */
  initial?: string;
  /** Searchable text beyond title/subtitle. */
  keywords?: string;
  /** Action to fire when this item is chosen. */
  run: () => void | Promise<void>;
  /** Computed at query time; renderer uses it for ordering within sections. */
  score?: number;
  /** Used by the streamer enrichment path: when the row becomes active, fetch
   *  description and re-render with `details` populated. */
  twitchUserId?: string;
  /** User-assigned shortcut (case-insensitive, single token). Matching this
   *  in the query gets a heavy score boost (above title-exact) so the user's
   *  aliases reliably trump fuzzy noise. */
  alias?: string;
  /** Marked true for snippets the user has favorited — surfaces a star icon
   *  and sorts the row to the top of its section. */
  favorite?: boolean;
  /** Current keyboard shortcut (display form, e.g. "Ctrl+Shift+D"), shown as a
   *  trailing chip. Populated for rows whose id matches a bindable command. */
  shortcut?: string;
}

// ---------- Clipboard helper ------------------------------------------------

/** Wrapper around `navigator.clipboard.writeText` matching the codebase
 *  convention (logService / StreamContextMenu / ThemeColorPicker all use it
 *  directly). Surfaces a toast on success/failure so the action gives obvious
 *  feedback in the palette flow. */
export async function copyToClipboard(text: string, successMessage?: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    useAppStore.getState().addToast(successMessage ?? 'Copied to clipboard', 'success');
  } catch (err) {
    Logger.warn('[CommandPalette] clipboard write failed:', err);
    useAppStore.getState().addToast('Copy failed', 'error');
  }
}

// ---------- Sleep timer (module-local) --------------------------------------

let sleepTimer: ReturnType<typeof setTimeout> | null = null;
let sleepTimerEnd = 0;

function setSleepTimer(minutes: number) {
  if (sleepTimer) clearTimeout(sleepTimer);
  sleepTimerEnd = Date.now() + minutes * 60_000;
  sleepTimer = setTimeout(() => {
    sleepTimer = null;
    sleepTimerEnd = 0;
    const store = useAppStore.getState();
    if (store.currentStream) void store.exitStream();
    store.addToast(`Sleep timer fired — stream stopped`, 'info');
  }, minutes * 60_000);
  useAppStore.getState().addToast(`Sleep timer set for ${minutes} minute${minutes === 1 ? '' : 's'}`, 'success');
}

function cancelSleepTimer() {
  if (!sleepTimer) {
    useAppStore.getState().addToast('No sleep timer active', 'info');
    return;
  }
  clearTimeout(sleepTimer);
  sleepTimer = null;
  sleepTimerEnd = 0;
  useAppStore.getState().addToast('Sleep timer cancelled', 'success');
}

function sleepTimerSubtitle(minutes: number): string {
  if (!sleepTimer) return `Auto-stop stream in ${minutes} minute${minutes === 1 ? '' : 's'}`;
  const remainingMin = Math.max(0, Math.round((sleepTimerEnd - Date.now()) / 60_000));
  return `Currently ${remainingMin}m remaining · click to reset to ${minutes}m`;
}

// ---------- Quick actions ---------------------------------------------------

function streamSocialUrl(login: string, segment: 'schedule' | ''): string {
  const base = `https://www.twitch.tv/${login}`;
  // Only `schedule` left after the native-path swap; others (about, videos,
  // clips) are now handled in-app via `cs.openStreamerProfile`,
  // `cs.watchLatestVod`, `cs.watchTopClip`.
  if (segment === 'schedule') return `${base}/schedule`;
  return base;
}

function requireStream(): TwitchStream | null {
  const stream = useAppStore.getState().currentStream;
  if (!stream || !stream.user_login) {
    useAppStore.getState().addToast('No active stream', 'warning');
    return null;
  }
  return stream;
}

/** One row per platform context, so the seam's menu has a keyboard equivalent.
 *  The subtitle carries the same live count the seam shows, which is what makes
 *  the row worth choosing between rather than a bare rename. */
function buildPlatformItems(): PaletteItem[] {
  if (WATCHABLE_PROVIDERS.length < 2) return [];
  const followed = useAppStore.getState().followedStreams.length;
  const live = Object.values(useFollowsStore.getState().liveByKey).filter((r) => r.is_live);
  const countOf = (p: ProviderId | 'all'): number => {
    if (p === 'twitch') return followed;
    if (p !== 'all') return live.filter((r) => r.provider === p).length;
    return WATCHABLE_PROVIDERS.reduce(
      (n, id) => n + (id === 'twitch' ? followed : live.filter((r) => r.provider === id).length),
      0,
    );
  };
  const options: (ProviderId | 'all')[] = ['all', ...WATCHABLE_PROVIDERS];
  return options.map((p) => {
    const label = p === 'all' ? 'All platforms' : providerLabel(p);
    return {
      id: `qa.platform.${p}`,
      section: 'Quick Actions' as const,
      title: `Switch to ${label}`,
      subtitle: `${countOf(p)} followed live`,
      keywords: `platform provider switch scope ${label.toLowerCase()} ${p}`,
      run: () => useAppStore.getState().setActivePlatform(p),
    };
  });
}

function buildQuickActions(): PaletteItem[] {
  const items: PaletteItem[] = [];

  // ---- Verb-style global actions
  items.push(
    {
      id: 'qa.openMultiChat',
      section: 'Quick Actions',
      title: 'Open MultiChat',
      subtitle: 'Spawn a new chat-only popout window',
      keywords: 'multichat popout chat window new',
      run: () => spawnMultiChat({}).catch((err) => Logger.error('[CommandPalette] openMultiChat failed:', err)),
    },
    {
      id: 'qa.openDrops',
      section: 'Quick Actions',
      title: 'Open Drops center',
      subtitle: 'Active campaigns, claimed rewards, automation status',
      keywords: 'drops campaigns rewards automation inventory',
      run: () => useAppStore.getState().setShowDropsOverlay(true),
    },
    {
      id: 'qa.openBadges',
      section: 'Quick Actions',
      title: 'Open Badges & Paints',
      subtitle: 'Browse Twitch + 7TV cosmetics catalog',
      keywords: 'badges paints cosmetics 7tv twitch',
      run: () => useAppStore.getState().setShowBadgesOverlay(true),
    },
    {
      id: 'qa.openEmoteSets',
      section: 'Quick Actions',
      title: 'Open 7TV Emotes',
      subtitle: 'Manage 7TV emotes, sets, and editors for channels you edit',
      keywords: 'emotes 7tv emote sets manage add remove rename editor channel',
      run: () => useAppStore.getState().openEmoteSets(),
    },
    {
      id: 'qa.openStreamNookTab',
      section: 'Quick Actions',
      title: 'Open StreamNook badge tab',
      subtitle: 'See your rank, tier, and identity surface',
      keywords: 'streamnook rank tier number identity ethereal mythic',
      run: () => useAppStore.getState().openBadgesOnStreamNook(),
    },
    {
      id: 'qa.openWhispers',
      section: 'Quick Actions',
      title: 'Open Whispers',
      subtitle: 'DMs imported from Twitch',
      keywords: 'whispers dms messages inbox',
      run: () => useAppStore.getState().setShowWhispersOverlay(true),
    },
    {
      id: 'qa.openProfile',
      section: 'Quick Actions',
      title: 'Open your profile',
      subtitle: 'Account, badges, StreamNook identity',
      keywords: 'profile account me',
      run: () => useAppStore.getState().openSettings('Profile'),
    },
    {
      id: 'qa.clearSearchHistory',
      section: 'Quick Actions',
      title: 'Clear search history',
      subtitle: 'Forget all remembered searches across every view',
      keywords: 'clear search history recent searches wipe delete forget remove following discover categories',
      run: () => {
        clearAllRecentSearches();
        useAppStore.getState().addToast('Search history cleared', 'success');
      },
    },
    {
      id: 'window.toggleFullscreen',
      section: 'Quick Actions',
      title: 'Toggle full screen',
      subtitle: 'Fill the whole screen (over the taskbar) with the app, chat and all',
      keywords: 'fullscreen full screen borderless taskbar immersive window maximize theater theatre cinema',
      run: () => useAppStore.getState().toggleWindowFullscreen(),
    },
    {
      id: 'window.toggleKeepOnTop',
      section: 'Quick Actions',
      title: 'Keep compact player on top',
      subtitle: 'Float the Compact View player above other apps',
      keywords: 'always on top pin float above stay in front behind browser buried cover overlay compact mini player',
      run: () => {
        const state = useAppStore.getState();
        if (!state.isTheaterMode) {
          state.addToast('Enter Compact View first', 'warning');
          return;
        }
        return state.toggleKeepOnTop();
      },
    },
    {
      id: 'qa.goHome',
      section: 'Quick Actions',
      title: 'Go to Home',
      subtitle: 'Followed, recommended, browse, search',
      keywords: 'home following recommended browse',
      run: () => {
        const store = useAppStore.getState();
        if (!store.isHomeActive) store.toggleHome();
      },
    },
    {
      id: 'qa.openSettings',
      section: 'Quick Actions',
      title: 'Open Settings',
      subtitle: 'All preferences',
      keywords: 'settings preferences options config',
      run: () => useAppStore.getState().openSettings(),
    },
    {
      id: 'qa.openWhatsNew',
      section: 'Quick Actions',
      title: "What's New",
      subtitle: 'Release log + component bumps',
      keywords: 'whatsnew updates changelog release notes',
      run: () => useAppStore.getState().openSettings("What's New"),
    },
    {
      id: 'qa.openPaletteWiki',
      section: 'Quick Actions',
      title: 'Command Palette · Guide & Snippet Manager',
      subtitle: 'Browse every feature; manage favorites, custom snippets, aliases',
      keywords: 'command palette guide help wiki snippets favorites alias manager docs',
      run: () => useAppStore.getState().openSettings('Command Palette'),
    },
    {
      id: 'qa.refreshFollows',
      section: 'Quick Actions',
      title: 'Refresh followed streams',
      subtitle: 'Re-pull the live following list from Twitch',
      keywords: 'refresh reload follows followed live update',
      run: () => useAppStore.getState().loadFollowedStreams(),
    },
    {
      id: 'qa.surpriseMe',
      section: 'Quick Actions',
      title: 'Surprise me, a random live follow',
      subtitle: 'Jump into a random channel from your following',
      keywords: 'random shuffle surprise lucky pick',
      run: () => {
        const followed = useAppStore.getState().followedStreams;
        if (followed.length === 0) {
          useAppStore.getState().addToast('No live channels to choose from', 'warning');
          return;
        }
        const pick = followed[Math.floor(Math.random() * followed.length)];
        return useAppStore.getState().startStream(pick.user_login, pick);
      },
    },
    {
      id: 'qa.topFollow',
      section: 'Quick Actions',
      title: 'Watch your top live follow',
      subtitle: 'Highest viewer count in your following right now',
      keywords: 'top biggest most viewers follow',
      run: () => {
        const followed = [...useAppStore.getState().followedStreams];
        if (followed.length === 0) {
          useAppStore.getState().addToast('No live follows right now', 'warning');
          return;
        }
        followed.sort((a, b) => (b.viewer_count || 0) - (a.viewer_count || 0));
        const top = followed[0];
        return useAppStore.getState().startStream(top.user_login, top);
      },
    },
  );

  // ---- Stream-context actions
  items.push(
    {
      id: 'cs.popoutCurrent',
      section: 'Current Stream',
      title: 'Pop out current chat to MultiChat',
      subtitle: "Move this stream's chat into a popout window",
      keywords: 'popout pop out chat current stream',
      run: () => {
        const stream = requireStream();
        if (!stream) return;
        return spawnMultiChat({
          channel: stream.user_login,
          channelId: stream.user_id || undefined,
          channelName: stream.user_name,
        });
      },
    },
    {
      id: 'cs.toggleTheatre',
      section: 'Current Stream',
      title: 'Toggle theatre mode',
      subtitle: 'Hide sidebar + chat for full-window video',
      keywords: 'theatre theater mode hide sidebar',
      run: () => useAppStore.getState().toggleTheaterMode(),
    },
    {
      id: 'cs.restartStream',
      section: 'Current Stream',
      title: 'Restart current stream',
      subtitle: 'Stop and re-fetch the active stream',
      keywords: 'restart refresh stream reload',
      run: () => {
        if (!requireStream()) return;
        return useAppStore.getState().restartStream();
      },
    },
    {
      id: 'cs.stopStream',
      section: 'Current Stream',
      title: 'Stop current stream',
      subtitle: 'Exit the stream and return home',
      keywords: 'stop exit close stream leave',
      run: () => {
        if (!useAppStore.getState().currentStream) return;
        return useAppStore.getState().exitStream();
      },
    },
    {
      id: 'cs.toggleFavorite',
      section: 'Current Stream',
      title: 'Toggle favorite streamer',
      subtitle: 'Keep this channel in your Favorites, followed or not',
      keywords: 'favorite fav pin star current watchlist',
      run: async () => {
        const stream = requireStream();
        if (!stream) return;
        // `favoriteIdOf`, not `user_id`: platform ids collide across services.
        const id = favoriteIdOf(stream);
        if (!id) return;
        await useAppStore.getState().toggleFavoriteStreamer(id, favoriteMetaOf(stream, id));
      },
    },
    {
      id: 'cs.followCurrent',
      section: 'Current Stream',
      title: 'Follow current channel',
      subtitle: 'Add this channel to your Twitch follows',
      keywords: 'follow current channel twitch',
      run: async () => {
        const stream = requireStream();
        if (!stream?.user_id) return;
        try {
          await invoke('follow_channel', { targetUserId: stream.user_id });
          useAppStore.getState().addToast(`Following ${stream.user_name}`, 'success');
        } catch (e) {
          Logger.warn('[CommandPalette] follow_channel failed:', e);
          useAppStore.getState().addToast('Follow failed', 'error');
        }
      },
    },
    {
      id: 'cs.unfollowCurrent',
      section: 'Current Stream',
      title: 'Unfollow current channel',
      subtitle: 'Remove this channel from your Twitch follows',
      keywords: 'unfollow current channel twitch',
      run: async () => {
        const stream = requireStream();
        if (!stream?.user_id) return;
        try {
          await invoke('unfollow_channel', { targetUserId: stream.user_id });
          useAppStore.getState().addToast(`Unfollowed ${stream.user_name}`, 'success');
        } catch (e) {
          Logger.warn('[CommandPalette] unfollow_channel failed:', e);
          useAppStore.getState().addToast('Unfollow failed', 'error');
        }
      },
    },
    {
      id: 'cs.dropsForGame',
      section: 'Current Stream',
      title: 'View drops for current game',
      subtitle: "Open Drops, filtered to this stream's game",
      keywords: 'drops current game campaigns this',
      run: () => {
        const stream = requireStream();
        if (!stream) return;
        const game = stream.game_name?.trim();
        if (!game) {
          useAppStore.getState().addToast('No game category on current stream', 'warning');
          return;
        }
        useAppStore.getState().openDropsWithSearch(game);
      },
    },
    {
      id: 'cs.browseCurrentGame',
      section: 'Current Stream',
      title: 'Browse other streams of this game',
      subtitle: "Jump to the Home category page for this stream's game",
      keywords: 'browse current game category other streams',
      run: async () => {
        const stream = requireStream();
        if (!stream?.game_name) {
          useAppStore.getState().addToast('No game category on current stream', 'warning');
          return;
        }
        await useAppStore.getState().navigateToCategoryByName(stream.game_name);
      },
    },
  );

  // ---- Native in-app current-stream navigation (NOT external) ---------
  // These three used to be browser-tab openers ("Open VODs on twitch.tv",
  // "Open About on twitch.tv", "Open Clips on twitch.tv"). StreamNook is
  // itself a Twitch app — leaving to twitch.tv for these is a cop-out. They
  // now resolve in-app via the native modals and player.
  items.push(
    {
      id: 'cs.openStreamerProfile',
      section: 'Current Stream',
      title: "Open this streamer's profile",
      subtitle: 'In-app modal — bio, panels, socials, follower count',
      keywords: 'about profile bio panels socials current streamer modal',
      run: () => {
        const stream = requireStream();
        if (!stream) return;
        useAppStore.getState().setProfileModalUser(stream);
      },
    },
    {
      id: 'cs.createClip',
      section: 'Current Stream',
      title: 'Create clip',
      subtitle: 'Clip ~30s of what you are watching (live, or a VOD at the current spot)',
      keywords: 'clip create capture moment highlight save vod',
      run: () => {
        void useAppStore.getState().createClip();
      },
    },
    {
      id: 'cs.watchLatestVod',
      section: 'Current Stream',
      title: "Watch this streamer's latest VOD",
      subtitle: 'Fetches the most recent archive and plays it in-app',
      keywords: 'vod video latest watch past broadcast archive replay',
      run: async () => {
        const stream = requireStream();
        if (!stream?.user_id) return;
        try {
          const [vods] = (await invoke('get_user_videos', {
            userId: stream.user_id,
            sort: 'time',
            limit: 1,
          })) as [TwitchVideo[], string | null];
          const latest = vods?.[0];
          if (!latest) {
            useAppStore.getState().addToast(`${stream.user_name} has no archived VODs`, 'info');
            return;
          }
          await useAppStore.getState().playMedia('video', latest.url, {
            id: latest.id,
            user_id: latest.user_id,
            user_name: latest.user_name,
            title: latest.title,
            view_count: latest.view_count,
            thumbnail_url: latest.thumbnail_url,
            created_at: latest.created_at,
            language: latest.language,
          });
        } catch (e) {
          Logger.warn('[CommandPalette] get_user_videos failed:', e);
          useAppStore.getState().addToast('Could not load VODs', 'error');
        }
      },
    },
    {
      id: 'cs.watchTopClip',
      section: 'Current Stream',
      title: "Watch this streamer's top clip",
      subtitle: 'Pulls the highest-viewed clip and plays it in-app',
      keywords: 'clip top best highest viewed watch current streamer',
      run: async () => {
        const stream = requireStream();
        if (!stream?.user_id) return;
        try {
          // Rust makes the Helix clips read (helix_get); the page never sees a token.
          const data = await helixGet<{ data?: TwitchClip[] }>('clips', `broadcaster_id=${encodeURIComponent(stream.user_id)}&first=1`);
          const top = data?.data?.[0];
          if (!top) {
            useAppStore.getState().addToast(`${stream.user_name} has no clips yet`, 'info');
            return;
          }
          await useAppStore.getState().playMedia('clip', top.url, {
            id: top.id,
            broadcaster_id: top.broadcaster_id,
            broadcaster_name: top.broadcaster_name,
            title: top.title,
            view_count: top.view_count,
            thumbnail_url: top.thumbnail_url,
            created_at: top.created_at,
            game_id: top.game_id,
            language: top.language,
            // Chat-replay coordinates. Helix supplies these natively; dropping them
            // here is what used to leave a palette-played clip without chat.
            clip_source: clipSourceOf(top),
          });
        } catch (e) {
          Logger.warn('[CommandPalette] top clip fetch failed:', e);
          useAppStore.getState().addToast('Could not load clips', 'error');
        }
      },
    },
  );

  // ---- Share / clipboard actions (current stream)
  // These genuinely move data OUT of StreamNook (to clipboard or OS browser)
  // — they're not lazy "open in browser" stand-ins for missing native UI.
  items.push(
    {
      id: 'sh.copyUrl',
      section: 'Share',
      title: 'Copy stream URL',
      subtitle: 'twitch.tv/<channel>',
      keywords: 'copy url link twitch share current',
      run: () => {
        const stream = requireStream();
        if (!stream) return;
        return copyToClipboard(`https://www.twitch.tv/${stream.user_login}`, `Copied twitch.tv/${stream.user_login}`);
      },
    },
    {
      id: 'sh.copyMarkdown',
      section: 'Share',
      title: 'Copy as markdown link',
      subtitle: '[DisplayName](https://twitch.tv/login)',
      keywords: 'copy markdown md link discord reddit',
      run: () => {
        const stream = requireStream();
        if (!stream) return;
        const md = `[${stream.user_name}](https://www.twitch.tv/${stream.user_login})`;
        return copyToClipboard(md, 'Copied markdown link');
      },
    },
    {
      id: 'sh.copyShareText',
      section: 'Share',
      title: 'Copy share text',
      subtitle: '"Watching X streaming Y — twitch.tv/X"',
      keywords: 'copy share tweet message twitter discord',
      run: () => {
        const stream = requireStream();
        if (!stream) return;
        const game = stream.game_name ? ` streaming ${stream.game_name}` : '';
        const text = `Watching ${stream.user_name}${game} on Twitch — https://www.twitch.tv/${stream.user_login}`;
        return copyToClipboard(text, 'Copied share text');
      },
    },
    {
      id: 'sh.copyEmbed',
      section: 'Share',
      title: 'Copy embed iframe HTML',
      subtitle: 'For pasting into a website or forum',
      keywords: 'copy embed iframe html parent website',
      run: () => {
        const stream = requireStream();
        if (!stream) return;
        const html = `<iframe src="https://player.twitch.tv/?channel=${stream.user_login}&parent=yourdomain.com" height="378" width="620" allowfullscreen></iframe>`;
        return copyToClipboard(html, 'Copied iframe HTML (replace parent=)');
      },
    },
    {
      id: 'sh.openTwitch',
      section: 'Share',
      title: 'Open current stream on twitch.tv',
      // Explicit "leave StreamNook" — useful when you want the full Twitch web
      // UI (e.g. extensions, sub gifting, prediction history). Not a stand-in
      // for missing native UI; native is `cs.openStreamerProfile` etc above.
      subtitle: 'Launches in your OS default browser (explicit external)',
      keywords: 'open browser twitch web current stream external',
      run: async () => {
        const stream = requireStream();
        if (!stream) return;
        try {
          await invoke('open_browser_url', { url: `https://www.twitch.tv/${stream.user_login}` });
        } catch (e) {
          Logger.warn('[CommandPalette] open_browser_url failed:', e);
        }
      },
    },
    {
      id: 'sh.openSchedule',
      section: 'Share',
      // Schedule is the lone holdout — no native schedule view exists in
      // StreamNook yet. If/when one ships, swap this for the native path.
      title: "Open this streamer's schedule on twitch.tv",
      subtitle: 'No native schedule view yet — opens in browser',
      keywords: 'schedule go live time when external',
      run: async () => {
        const stream = requireStream();
        if (!stream) return;
        try {
          await invoke('open_browser_url', { url: streamSocialUrl(stream.user_login, 'schedule') });
        } catch (e) {
          Logger.warn('[CommandPalette] open_browser_url failed:', e);
        }
      },
    },
  );

  // ---- Sleep timer
  items.push(
    {
      id: 'qa.sleep15',
      section: 'Quick Actions',
      title: 'Sleep timer · 15 minutes',
      subtitle: sleepTimerSubtitle(15),
      keywords: 'sleep timer 15 minutes auto stop',
      run: () => setSleepTimer(15),
    },
    {
      id: 'qa.sleep30',
      section: 'Quick Actions',
      title: 'Sleep timer · 30 minutes',
      subtitle: sleepTimerSubtitle(30),
      keywords: 'sleep timer 30 minutes auto stop',
      run: () => setSleepTimer(30),
    },
    {
      id: 'qa.sleep60',
      section: 'Quick Actions',
      title: 'Sleep timer · 1 hour',
      subtitle: sleepTimerSubtitle(60),
      keywords: 'sleep timer 60 minutes 1 hour auto stop',
      run: () => setSleepTimer(60),
    },
    {
      id: 'qa.sleep120',
      section: 'Quick Actions',
      title: 'Sleep timer · 2 hours',
      subtitle: sleepTimerSubtitle(120),
      keywords: 'sleep timer 120 minutes 2 hours auto stop',
      run: () => setSleepTimer(120),
    },
    {
      id: 'qa.sleepCancel',
      section: 'Quick Actions',
      title: 'Cancel sleep timer',
      subtitle: 'Stops any scheduled auto-stop',
      keywords: 'cancel clear sleep timer off',
      run: () => cancelSleepTimer(),
    },
  );

  // ---- Help / feedback links
  items.push(
    {
      id: 'qa.githubIssue',
      section: 'Quick Actions',
      title: 'Report an issue on GitHub',
      subtitle: 'Opens the StreamNook issues page in your browser',
      keywords: 'github issue bug report feedback help',
      run: async () => {
        try {
          await invoke('open_browser_url', { url: 'https://github.com/winters27/StreamNook/issues/new' });
        } catch (e) {
          Logger.warn('[CommandPalette] open_browser_url failed:', e);
        }
      },
    },
    {
      id: 'qa.githubRepo',
      section: 'Quick Actions',
      title: 'Open StreamNook on GitHub',
      subtitle: 'Source, releases, discussions',
      keywords: 'github repo source code repository',
      run: async () => {
        try {
          await invoke('open_browser_url', { url: 'https://github.com/winters27/StreamNook' });
        } catch (e) {
          Logger.warn('[CommandPalette] open_browser_url failed:', e);
        }
      },
    },
  );

  return items;
}

// ---------- Settings catalog ------------------------------------------------

interface SettingsEntry {
  tab: SettingsTab;
  section?: string;
  /** Overrides the displayed title (defaults to `section`, then `tab`). Lets a
   *  single section expose several rows in the palette — one per control — so a
   *  specific toggle name lands on its own result. */
  label?: string;
  /** DOM id of the matching <SettingsSection> (or its wrapper), when one exists.
   *  Passed to openSettings so the palette scrolls to the section, not just the
   *  tab. Mirrors `sectionId` in src/components/settings/searchIndex.ts. */
  sectionId?: string;
  keywords?: string;
}

// Manual catalog for the Ctrl+K palette. One entry per Settings tab + per
// real <SettingsSection>; `section` must match the section's label and
// `sectionId` its DOM id (so the palette scrolls to it, not just the tab).
// `keywords` is the only match surface here, so pack synonyms in. Keep in sync
// with SETTINGS_INDEX in src/components/settings/searchIndex.ts.
// Profile and Plugins are intentionally absent: Profile uses no <SettingsSection>
// primitives, and Plugins is not rendered by SettingsDialog (it lives in the
// Marketplace overlay), so openSettings('Plugins') would show a blank tab.
const SETTINGS_CATALOG: SettingsEntry[] = [
  // Player
  { tab: 'Player', keywords: 'player video stream playback overlay buttons auto switch streaming codecs audio boost song id' },
  { tab: 'Player', section: 'Player Overlay Buttons', keywords: 'player overlay buttons follow subscribe clip identify song clips vods multinook refresh close hide show customize which buttons top right' },
  { tab: 'Player', section: 'Mouse Controls', sectionId: 'settings-section-mouse-controls', keywords: 'mouse wheel scroll volume louder quieter middle click mute unmute one handed scroll wheel button volume step notch channel about reveal shift scroll down' },
  { tab: 'Player', section: 'Auto-Switch', sectionId: 'settings-section-auto-switch', keywords: 'auto switch fallback offline next stream raid redirect followed category notification stay offline chat' },
  { tab: 'Player', section: 'Streaming', sectionId: 'settings-section-streaming', keywords: 'streaming codecs h265 hevc av1 h264 connection timeout auto retry delay resolve' },
  { tab: 'Player', section: 'Video Player', sectionId: 'settings-section-video-player', keywords: 'video player autoplay live edge gap low latency buffer quality volume aspect ratio lock start muted fullscreen controls cinema mode letterbox bars black color match theme floating pillarbox immersive' },
  { tab: 'Player', section: 'Audio Boost', sectionId: 'settings-section-audio-boost', keywords: 'audio boost compressor makeup gain louder volume normalize even out loud quiet clipping threshold ratio knee attack release' },
  { tab: 'Player', section: 'Song Identification', sectionId: 'settings-section-song-id', keywords: 'song identification identify music what song is this shazam recognize now playing track name spotify apple music song.link listen time retries capture detection' },

  // Theme
  { tab: 'Theme', keywords: 'theme color accent skin dark light palette glassiness font' },
  { tab: 'Theme', section: 'Glassiness', keywords: 'glassiness glass transparency frosted opacity panels see-through blur solid flat opaque disable turn off no glass' },
  { tab: 'Theme', section: 'Font', sectionId: 'settings-section-font', keywords: 'font typeface interface satoshi inter geist manrope outfit space grotesk serif system family text' },
  { tab: 'Theme', section: 'Font', sectionId: 'settings-section-font', label: 'Custom font', keywords: 'custom font own font any font google fonts poppins bebas neue rubik lato nunito playfair jetbrains mono quicksand typeface family install download add my own change font' },

  // Chat
  { tab: 'Chat', keywords: 'chat placement design fonts dividers timestamps mentions emotes logging channel points highlights commands reminders' },
  { tab: 'Chat', section: 'Chat Placement', keywords: 'chat placement position right bottom hidden where show hide' },
  { tab: 'Chat', section: 'Channel Points', keywords: 'channel points auto claim bonus chest reward collect points' },
  { tab: 'Chat', section: 'YouTube Chat', sectionId: 'settings-section-youtube-chat', label: 'Which chat to read', keywords: 'youtube chat view live chat top chat filtered firehose every message flooding flood too fast too many messages slow down readable quality filter' },
  { tab: 'Chat', section: 'Chat Events', sectionId: 'settings-section-chat-events', keywords: 'chat events live activity overlay show hide toggle turn off in chat' },
  { tab: 'Chat', section: 'Chat Events', sectionId: 'settings-section-chat-events', label: 'Polls', keywords: 'polls poll vote voting live poll overlay chat event show hide toggle turn off' },
  { tab: 'Chat', section: 'Chat Events', sectionId: 'settings-section-chat-events', label: 'Predictions', keywords: 'predictions prediction bet outcome channel points overlay chat event show hide toggle turn off' },
  { tab: 'Chat', section: 'Chat Events', sectionId: 'settings-section-chat-events', label: 'Poll and prediction order', keywords: 'poll prediction both at once same time overlap stack order priority which on top first card overlay position' },
  { tab: 'Chat', section: 'Chat Events', sectionId: 'settings-section-chat-events', label: 'Channel Point Redemptions', keywords: 'channel point redemptions redeem reward no input chat row show hide toggle turn off' },
  { tab: 'Chat', section: 'Chat Events', sectionId: 'settings-section-chat-events', label: 'Collapse gift-sub floods', keywords: 'gift subs gifted collapse flood mystery bomb community batch spam one liner recipients sub gift show hide toggle turn off' },
  { tab: 'Chat', section: 'Chat Events', sectionId: 'settings-section-chat-events', label: 'Chat replay on clips', keywords: 'clip chat replay historical past chat vod comments beside clip player show hide toggle' },
  { tab: 'Chat', section: 'Chat Logging', keywords: 'chat logging save logs text files folder per channel timestamps events moderation record history' },
  { tab: 'Chat', section: 'Chat Design', keywords: 'chat design font size weight spacing dividers timestamps seconds mention colors reply name separator style prefix colon dot arrow pipe dash chip bracket accent bar pinned message collapse bar alternating backgrounds' },
  { tab: 'Chat', section: 'Link Previews', keywords: 'link preview previews load card url unfurl embed trusted sources shorten links domains clean' },
  { tab: 'Chat', section: 'Emotes', keywords: 'emotes emote size hover preview spacing inline scale 7tv bttv ffz' },
  { tab: 'Chat', section: 'Chat Input', keywords: 'chat input composer bypass duplicate message quick send ctrl enter keep message repeat' },
  { tab: 'Chat', section: 'Chat Input', label: 'Check spelling as you type', keywords: 'spell check spelling typo autocorrect correction misspelled underline squiggle red wavy dictionary proofread grammar' },
  { tab: 'Chat', section: 'Chat Input', label: 'Spell check dictionary', keywords: 'dictionary custom words add word taught spell check spelling ignore list personal vocabulary' },
  { tab: 'Chat', section: 'Chat Input', label: 'Hide the placeholder text', keywords: 'hide placeholder prompt send a message empty input box composer text grey ghost hint clean minimal' },
  { tab: 'Chat', section: 'Chat Input', label: 'Hide the emote button', keywords: 'hide emote emoji button smiley face icon input box composer picker clean minimal remove' },
  { tab: 'Chat', section: 'Chat Input', label: 'Hide the points balance', keywords: 'hide channel points balance number counter button input box composer clean minimal remove' },
  { tab: 'Chat', section: 'Emote Tab Completion', sectionId: 'settings-section-emote-tab-completion', keywords: 'emote tab completion autocomplete carousel kappa cycle shift starts contains match include chat users' },
  { tab: 'Chat', section: 'Render Style', keywords: 'render style deleted messages strikethrough dimmed hidden shared chat paint mentions inline compact emote tooltips 7tv update notices smooth scroll resume message buffer scrollback ffz emote effects modifier wide flip rainbow shake frankerfacez bttv betterttv emote modifiers cursed party rotate zero space giant emotes gigantify gigantified power-up powerup big huge large' },
  { tab: 'Chat', section: 'Chat Events', sectionId: 'settings-section-chat-events', label: 'Start a poll or prediction', keywords: 'poll prediction create start new make run builder composer outcomes choices duration channel points vote bet broadcaster streamer' },
  { tab: 'Chat', section: 'Hidden Users & Bots', sectionId: 'settings-section-chat-filters', keywords: 'hide hidden users bots filter block blocklist mute ignore streamelements nightbot moobot fossabot bot spam gamble flood chat filters per channel everywhere global unhide' },
  { tab: 'Chat', section: 'Repeated Messages', sectionId: 'settings-section-repeated-messages', keywords: 'repeated messages repeat counter duplicate copypasta spam wave combo collapse fold group x2 x3 x12 count same message emote flood dedupe condense' },
  { tab: 'Chat', section: 'Repeated Messages', sectionId: 'settings-section-repeated-messages', label: 'Repeat counter colour and threshold', keywords: 'repeat counter colour color threshold window seconds sensitivity match exact nearly identical mods vips streamer exempt moderator' },
  { tab: 'Chat', section: 'User Cards', sectionId: 'settings-section-user-cards', keywords: 'user card profile popup click username open messages chat history first badges stats view default landing' },
  { tab: 'Chat', section: 'User Cards', sectionId: 'settings-section-user-cards', label: 'User card details', keywords: 'user card fields rows details show hide joined twitch account age followage following since follows channels chatters chatter count past subscriber last live relative time how long ago 7tv profile link banned suspended' },
  { tab: 'Chat', section: '7TV Cosmetics', keywords: '7tv cosmetics paint drop shadows username paints shadow readability' },
  { tab: 'Chat', section: 'Highlight Appearance', keywords: 'highlight appearance display style tint opacity flash window title unfocused look' },
  { tab: 'Chat', section: 'Highlight Phrases', keywords: 'highlights phrases keywords alerts words names patterns flash match' },
  { tab: 'Chat', section: 'Built-in Event Highlights', keywords: 'built-in event highlights first-time chatters returning your own messages raid announcements auto highlight' },
  { tab: 'Chat', section: 'Username Highlights', keywords: 'username highlights highlight user login by name case-insensitive' },
  { tab: 'Chat', section: 'Badge Highlights', keywords: 'badge highlights highlight by badge moderator vip subscriber name version' },
  { tab: 'Chat', section: 'Custom Commands', keywords: 'custom commands slash macros expansions auto-fill' },
  { tab: 'Chat', section: 'Reminders', sectionId: 'reminders', keywords: 'reminder reminders remind auto message timer schedule recurring interval keyword nudge streamer repeat post chat uptime delay' },
  { tab: 'Chat', section: 'User Overrides', keywords: 'user overrides nicknames per-user nickname rename chatter' },

  // Moderation
  { tab: 'Moderation', keywords: 'moderation mod ban timeout delete chat actions logs nuke purge mass visibility highlights' },
  { tab: 'Moderation', section: 'Reasons', sectionId: 'settings-section-mod-reasons', label: 'Ban and nuke reasons', keywords: 'reason reasons ban timeout nuke saved preset prefill default text why mod view record note' },
  { tab: 'Moderation', section: 'Moderation Actions', keywords: 'moderation actions action style mod drag moderate grab buttons both ban timeout delete whisper profile buckets pin pinned message inline column bar above beside chat layout' },
  { tab: 'Moderation', section: 'Mod Rooms', keywords: 'mod rooms room private encrypted team chat consent connect disconnect connected account moderator backchannel' },
  { tab: 'Moderation', section: 'Mod Logs', keywords: 'mod logs panel recent moderation actions timeouts bans deletions sidebar' },
  { tab: 'Moderation', section: 'Message Visibility', keywords: 'message visibility announce mod actions inline hide strikethrough removed banned timed out deleted' },
  { tab: 'Moderation', section: 'Log Highlights', keywords: 'log highlights color code mod log severity highlight style category colors' },
  { tab: 'Moderation', section: 'Mass Actions', keywords: 'mass actions nuke undo regex phrase bulk purge sweep' },

  // Overlay
  { tab: 'Overlay', keywords: 'stream overlay chat overlay obs streamelements streamlabs browser source on-stream chat box widget multi-platform merged chat emotes 7tv paints badges cosmetics stream chat on screen' },
  { tab: 'Overlay', section: 'Sources', keywords: 'overlay sources platforms twitch kick youtube tiktok merge source tag which platform label dot' },
  { tab: 'Overlay', section: 'Typography', keywords: 'overlay font family size line height message spacing typography text justify align alignment left center centre right bold italic strikethrough weight light slant crossed out' },
  { tab: 'Overlay', section: 'Typography', label: 'Justify text', keywords: 'overlay justify align alignment text left center centre right middle position messages' },
  { tab: 'Overlay', section: 'Typography', label: 'Text style', keywords: 'overlay bold italic strikethrough font weight light regular medium semibold slant crossed out line through message text style' },
  { tab: 'Overlay', section: 'Emotes & Badges', keywords: 'overlay emote size badges show hide scale third-party 7tv ffz chatterino paints streamnook member badge atmosphere cosmetics disable' },
  { tab: 'Overlay', section: 'Emotes & Badges', label: 'Giant emote placement', keywords: 'overlay giant emote placement gigantify power up align left center right inline below next to username colon position big emote' },
  { tab: 'Overlay', section: 'Appearance', keywords: 'overlay appearance text color shadow legibility timestamps transparent solid background opacity scene size blur spread strength outline stroke drop shadow contrast readable' },
  { tab: 'Overlay', section: 'Chatters', label: 'Profile pictures', keywords: 'overlay profile pictures avatars pfp youtube tiktok chatter photo show hide toggle' },
  { tab: 'Overlay', section: 'Chatters', label: '@ before usernames', keywords: 'overlay at sign @ username handle youtube strip remove show hide toggle' },
  { tab: 'Overlay', section: 'Messages', label: 'Replies', keywords: 'overlay replying to reply context line thread mention at username old twitch style show hide toggle remove off' },
  { tab: 'Overlay', section: 'Messages', label: 'Links', keywords: 'overlay link url hyperlink blue accent color underline plain body text no underline style' },
  { tab: 'Overlay', section: 'Emotes & Badges', label: '7TV personal emotes', keywords: 'overlay 7tv personal emotes subscriber own set every channel unknown random emote not added show hide toggle' },
  { tab: 'Overlay', section: 'Events', label: 'Custom event text', keywords: 'overlay custom event text message template wording tokens variables legend reference username months years streak tier recipient count bits viewers charity amount goal points channel platform time action resub sub gift raid milestone follow announcement rewrite greeting welcome' },
  { tab: 'Overlay', section: 'Chatters', label: 'First-time chatters', keywords: 'overlay first time chatter first message highlight outline border new chatter twitch streamnook style purple pink show hide toggle off' },
  { tab: 'Overlay', section: 'Chatters', label: 'Fill the highlight', keywords: 'overlay first time chatter fill tint background highlight transparent color matched' },
  { tab: 'Overlay', section: 'Chatters', label: 'First-time highlight animation', keywords: 'overlay first time chatter animate animation sheen pulse chase sweep shimmer border flash spark highlight one shot repeat loop every 5 seconds' },
  { tab: 'Overlay', section: 'Events', label: 'Event style', keywords: 'overlay event style plain outline streamnook ring tint gradient wash subs gifts raids design look' },
  { tab: 'Overlay', section: 'Events', label: 'Bits messages', keywords: 'overlay bits cheer message event card display inline promote subs style gem tier' },
  { tab: 'Overlay', section: 'Events', label: 'Show events', keywords: 'overlay show hide events per source per platform twitch youtube tiktok kick filter subs gifts raids bits cheer follows milestones announcements event type toggle' },
  { tab: 'Overlay', section: 'Events', label: 'Fill the outline', keywords: 'overlay event outline fill tint background transparent color matched ring' },
  { tab: 'Overlay', section: 'Chatters', label: 'Highlight color', keywords: 'overlay first time chatter highlight color custom accent pink purple recolor tint' },
  { tab: 'Overlay', section: 'Messages', label: 'Message bubbles', keywords: 'overlay message bubbles bubble pill speech messenger shape corner radius background rounded color opacity chat box' },
  { tab: 'Overlay', section: 'Messages', label: 'Max lines per message', keywords: 'overlay max lines truncate clamp long message copypasta ellipsis wall of text limit' },
  { tab: 'Overlay', section: 'Messages', label: 'Remove messages after', keywords: 'overlay remove expire auto clear stale old messages seconds lifetime hide after inactivity' },
  { tab: 'Overlay', section: 'Messages', label: 'Restore chat on reload', keywords: 'overlay restore keep clear chat on reload obs refresh restart stream start buffer persist blank empty last messages' },
  { tab: 'Overlay', section: 'Filters', label: 'Hide messages containing', keywords: 'overlay hide messages containing words phrases profanity filter banned blocklist spoiler swear' },
  { tab: 'Overlay', section: 'Events', label: 'Outline color', keywords: 'overlay event outline color fixed ring custom platform default recolor' },
  { tab: 'Overlay', section: 'Events', label: 'Event outline animation', keywords: 'overlay event outline animate animation sheen pulse chase sweep shimmer border flash spark ring one shot repeat loop every 5 seconds' },
  { tab: 'Overlay', section: 'Behavior', keywords: 'overlay behavior direction bottom top entrance animation fade slide drift rise pop stamp max messages cap' },

  // Interface
  { tab: 'Interface', keywords: 'interface sidebar motion animations settings window compact view chrome layout' },
  { tab: 'Interface', section: 'Sidebar', sectionId: 'settings-section-sidebar', keywords: 'sidebar nav navigation rail display mode expanded compact hidden disabled expand on hover recommended streams' },
  { tab: 'Interface', section: 'Discover Feed', sectionId: 'settings-section-discover', label: 'Languages', keywords: 'discover feed recommended streams languages language filter broadcast english french german spanish region locale' },
  { tab: 'Interface', section: 'Discover Feed', sectionId: 'settings-section-discover', label: 'Personalized recommendations', keywords: 'discover personalized recommendations anonymous privacy tracking account tailored opt in' },
  { tab: 'Interface', section: 'Motion', sectionId: 'settings-section-motion', keywords: 'motion animations reduce motion accessibility performance disable transitions full reduced off snappy' },
  { tab: 'Interface', section: 'Closing the Window', sectionId: 'settings-section-window-close', label: 'Close button', keywords: 'close button minimize to tray system tray close to tray quit exit x button background keep running notification area popouts' },
  { tab: 'Interface', section: 'Keep on Top', sectionId: 'settings-section-window-on-top', keywords: 'always on top pin keep window above float stay in front overlay floating player behind browser buried sinks disappears compact view mini player picture in picture alternative' },
  { tab: 'Interface', section: 'Settings Window', sectionId: 'settings-section-settings-window', keywords: 'settings window compact centered full page layout fills app' },
  { tab: 'Interface', section: 'Compact View', sectionId: 'settings-section-compact', keywords: 'compact view mini small window size second monitor preset' },

  // Profile — platform accounts live with the Twitch ones, not under Integrations.
  { tab: 'Profile', section: 'Accounts', label: 'Platform accounts', keywords: 'accounts kick youtube twitch connect disconnect sign in link platform add account multi-platform followed channels subscriptions' },

  // Integrations
  { tab: 'Integrations', keywords: 'integrations discord rpc rich presence ttv lol ad block ad-free connected apps services' },
  { tab: 'Integrations', section: 'Discord Rich Presence', keywords: 'discord rpc rich presence activity status what watching' },
  { tab: 'Integrations', section: 'Ad Blocking', keywords: 'ad block ad-free ads ttv lol proxy splice block twitch ads plugin' },

  // Notifications
  { tab: 'Notifications', keywords: 'notifications toast dynamic island sound alerts live whisper drops update channel points badge' },
  { tab: 'Notifications', section: 'Notification Methods', keywords: 'notification methods dynamic island toast position edge spacing corner' },
  { tab: 'Notifications', section: 'Notification Types', keywords: 'notification types live going live whisper dm update available drops channel points badge favorite favorites favorite channel watchlist category' },
  { tab: 'Notifications', section: 'Sound', keywords: 'sound notification sound style test ping audio' },

  // Cache
  { tab: 'Cache', keywords: 'cache clear storage expiry emote badge size maintenance prefetch' },
  { tab: 'Cache', section: 'Emote Prefetch', keywords: 'emote prefetch preload warm cache download all follows scan instant menu' },

  // Command Palette
  { tab: 'Command Palette', keywords: 'command palette ctrl k guide wiki snippets favorites alias manager docs' },
  { tab: 'Command Palette', section: 'Snippet Manager', sectionId: 'settings-section-snippets', keywords: 'snippets manager copypasta favorites aliases custom add star' },
  { tab: 'Command Palette', section: 'Keyboard Shortcuts', sectionId: 'settings-section-keyboard', keywords: 'keyboard shortcuts hotkeys ctrl k arrow keys enter esc home end' },
  { tab: 'Command Palette', section: 'What lives in the palette', keywords: 'what lives palette guide overview sections quick actions current stream share categories snippets' },

  // Keybindings
  { tab: 'Keybindings', keywords: 'keybindings keyboard shortcuts hotkeys binds combos rebind customize reset chord' },
  { tab: 'Keybindings', section: 'Application', keywords: 'application app-wide shortcuts everywhere global hotkeys' },
  { tab: 'Keybindings', section: 'Navigation', keywords: 'navigation jump surfaces shortcuts hotkeys' },
  { tab: 'Keybindings', section: 'Player', keywords: 'player shortcuts play pause mute fullscreen volume hotkeys while playing' },
  { tab: 'Keybindings', section: 'Moderation', keywords: 'moderation shortcuts focus message j k act ban timeout hotkeys' },
  { tab: 'Keybindings', section: 'Chat', keywords: 'chat shortcuts compose field hotkeys' },
  { tab: 'Keybindings', section: 'Multi-view', keywords: 'multi-view multichat windows shortcuts hotkeys' },

  // Backup
  { tab: 'Backup', keywords: 'backup restore export import settings file save preferences move new pc reinstall' },
  { tab: 'Backup', section: 'Backup and restore', keywords: 'backup restore export import settings file save load preferences' },
  { tab: 'Backup', section: 'Settings file', keywords: 'settings file folder open settings.json location on disk' },

  // Support
  { tab: 'Support', keywords: 'support help community discord join invite feature request updates' },
  { tab: 'Support', section: 'Community Discord', keywords: 'community discord join invite server help feature request' },
  { tab: 'Support', section: 'Diagnostics', keywords: 'diagnostics diagnostic log logs logging verbose debug error reporting bug report troubleshoot' },

  // What's New
  { tab: "What's New", keywords: 'whats new changelog release notes updates' },

  // Analytics (admin)
  { tab: 'Analytics', keywords: 'analytics dashboard users online stats supabase' },
];

function buildSettingsItems(): PaletteItem[] {
  return SETTINGS_CATALOG.map((entry) => {
    const title = entry.label ?? (entry.section ? entry.section : entry.tab);
    const subtitle = entry.section ? `Settings · ${entry.tab}` : 'Settings';
    return {
      id: `settings.${entry.tab}.${entry.section ?? '_overview'}${entry.label ? `.${entry.label}` : ''}`,
      section: 'Settings',
      title,
      subtitle,
      keywords: `${entry.tab.toLowerCase()} ${entry.keywords ?? ''}`.trim(),
      initial: title.slice(0, 1).toUpperCase(),
      run: () => useAppStore.getState().openSettings(entry.tab, entry.sectionId),
    };
  });
}

// ---------- Snippets --------------------------------------------------------

/** Convert a Snippet record into a PaletteItem. Pulls per-snippet favorite +
 *  alias state from the snippet store at call time so changes in the settings
 *  manager reflect on the next render without any explicit invalidation. */
function snippetToItem(s: Snippet, opts: { favorite: boolean; alias?: string }): PaletteItem {
  // Strip newlines for the subtitle preview so the row stays one-line tall;
  // the full content lives in `details` and is shown when the row is active.
  const preview = s.content.replace(/\s+/g, ' ').slice(0, 70);
  const aliasHint = opts.alias ? ` · ⌨ ${opts.alias}` : '';
  return {
    id: `snippet.${s.id}`,
    section: 'Snippets',
    title: s.title,
    subtitle: `${s.category}${aliasHint} · ${preview}${s.content.length > 70 ? '…' : ''}`,
    details: s.content,
    keywords: `${s.category} ${s.title} ${s.keywords ?? ''} ${s.content}`.toLowerCase(),
    initial: opts.favorite ? '★' : s.category.slice(0, 1).toUpperCase(),
    alias: opts.alias,
    favorite: opts.favorite,
    run: () => copyToClipboard(s.content, `Copied "${s.title}"`),
  };
}

/** Read built-in + user-custom snippets and project them as PaletteItems with
 *  favorites/aliases applied. Called from `getStaticItems()` so the palette
 *  picks up snippet-store changes on every render (which is when its memo
 *  reruns, since the palette subscribes to the store). */
export function getSnippetItems(): PaletteItem[] {
  const { customSnippets, favoriteIds, aliases } = useSnippetStore.getState();
  const all: Snippet[] = [...getBuiltInSnippets(), ...customSnippets];
  return all.map((s) =>
    snippetToItem(s, {
      favorite: favoriteIds.has(s.id),
      alias: aliases.get(s.id),
    }),
  );
}

// ---------- Plugin-contributed rows ------------------------------------------

/** Rows registered by ui plugins (src/plugins-ui/). Pulled fresh per render,
 *  same approach as getSnippetItems, so rows a plugin derives from its own
 *  data stay current. A failing provider drops only its own rows. */
export function getPluginPaletteItems(): PaletteItem[] {
  const items: PaletteItem[] = [];
  for (const { pluginId, provider } of usePluginUiRegistry.getState().paletteProviders) {
    try {
      items.push(
        ...provider().map((item) => ({
          section: 'Quick Actions' as const,
          ...item,
        })),
      );
    } catch (err) {
      Logger.warn(`[CommandPalette] palette provider from ${pluginId} failed:`, err);
    }
  }
  return items;
}

// Hotkey-driven actions that aren't otherwise in the static catalog — surfaced
// so every bindable action is reachable from the palette (and shows its
// hotkey). Player controls no-op gracefully when nothing is playing; the stream
// nav rows reuse the keybinding registry's handlers so the logic lives in one
// place.
function buildPlayerControlItems(): PaletteItem[] {
  const pc = () => getPlayerControls();
  const runCmd = (id: string) => () => getBindableCommand(id)?.run?.();
  return [
    { id: 'player.playPause', section: 'Current Stream', title: 'Play / pause', keywords: 'play pause resume video player', run: () => pc()?.togglePlay() },
    { id: 'player.mute', section: 'Current Stream', title: 'Mute / unmute', keywords: 'mute unmute audio sound volume', run: () => pc()?.toggleMute() },
    { id: 'player.fullscreen', section: 'Current Stream', title: 'Toggle fullscreen', keywords: 'fullscreen full screen video', run: () => pc()?.toggleFullscreen() },
    { id: 'player.pip', section: 'Current Stream', title: 'Picture-in-picture', keywords: 'pip picture in picture mini player', run: () => pc()?.togglePip() },
    { id: 'player.volumeUp', section: 'Current Stream', title: 'Volume up', keywords: 'volume up louder increase', run: () => pc()?.volumeUp() },
    { id: 'player.volumeDown', section: 'Current Stream', title: 'Volume down', keywords: 'volume down quieter decrease', run: () => pc()?.volumeDown() },
    { id: 'nav.nextStream', section: 'Quick Actions', title: 'Next followed stream', subtitle: 'Switch to the next live channel you follow', keywords: 'next followed channel switch surf cycle', run: runCmd('nav.nextStream') },
    { id: 'nav.prevStream', section: 'Quick Actions', title: 'Previous followed stream', subtitle: 'Switch to the previous live channel you follow', keywords: 'previous prev followed channel switch surf cycle', run: runCmd('nav.prevStream') },
  ];
}

export function getStaticItems(): PaletteItem[] {
  // Static-but-dynamic: quick actions + settings catalog are truly static,
  // sleep-timer subtitles are recomputed for live countdown text, snippets
  // are pulled fresh so the snippet store's favorites/custom/aliases changes
  // surface immediately, social links are pulled from the about-data cache
  // (one row per link for the current stream).
  const quick = buildQuickActions();
  const settings = buildSettingsItems();
  const snippets = getSnippetItems();
  const pluginItems = getPluginPaletteItems();
  const socials = getCurrentStreamSocialItems();
  const playerControls = buildPlayerControlItems();
  const platforms = buildPlatformItems();
  const items: PaletteItem[] = [
    ...platforms,
    ...quick.map((it) => {
      if (it.id === 'qa.sleep15') return { ...it, subtitle: sleepTimerSubtitle(15) };
      if (it.id === 'qa.sleep30') return { ...it, subtitle: sleepTimerSubtitle(30) };
      if (it.id === 'qa.sleep60') return { ...it, subtitle: sleepTimerSubtitle(60) };
      if (it.id === 'qa.sleep120') return { ...it, subtitle: sleepTimerSubtitle(120) };
      return it;
    }),
    ...settings,
    ...playerControls,
    ...snippets,
    ...pluginItems,
    ...socials,
  ];
  // Enrich rows whose id matches a bindable command with their current key
  // combo, so the palette doubles as a live shortcut reference.
  const shortcuts = getShortcutDisplayMap();
  return items.map((it) => (shortcuts[it.id] ? { ...it, shortcut: shortcuts[it.id] } : it));
}

// ---------- Followed channels (from AppStore) -------------------------------

function streamToItem(stream: TwitchStream): PaletteItem {
  const subtitleParts: string[] = [];
  if (stream.game_name) subtitleParts.push(stream.game_name);
  if (typeof stream.viewer_count === 'number' && stream.viewer_count > 0) {
    subtitleParts.push(`${stream.viewer_count.toLocaleString()} viewers`);
  }
  return {
    id: `stream.${stream.user_id || stream.user_login}`,
    section: 'Followed Channels',
    title: stream.user_name || stream.user_login,
    subtitle: subtitleParts.join(' · ') || 'Live',
    avatarUrl: stream.profile_image_url || undefined,
    initial: (stream.user_login || stream.user_name || '?').slice(0, 1).toUpperCase(),
    keywords: `${stream.user_login} ${stream.user_name} ${stream.game_name ?? ''}`.toLowerCase(),
    twitchUserId: stream.user_id || undefined,
    run: () => useAppStore.getState().startStream(stream.user_login, stream),
  };
}

export function getFollowedItems(): PaletteItem[] {
  const followed = useAppStore.getState().followedStreams;
  return followed.map(streamToItem);
}

// ---------- Current-stream social links (from cached about data) -----------

/** Heuristic name → initial for the avatar fallback. Cleaner than emoji
 *  guessing — the social link's first letter is unique enough at the row
 *  size, and the row's title carries the full name. */
function socialInitial(link: SocialMediaLink): string {
  const n = (link.name || link.title || '?').trim();
  return n.slice(0, 1).toUpperCase();
}

/** Build palette rows for each social link on the current stream's about
 *  payload, if it's been fetched (lazy + cached via `warmupAbout`). Returns
 *  empty if no current stream OR if the about data hasn't landed yet — the
 *  rows will materialize on the next render after the warmup resolves. */
export function getCurrentStreamSocialItems(): PaletteItem[] {
  const stream = useAppStore.getState().currentStream;
  if (!stream?.user_id) return [];
  const about = getCachedAbout(stream.user_id);
  if (!about || !about.social_links?.length) return [];
  return about.social_links.map((link) => ({
    id: `cs.social.${link.name}.${link.url}`,
    section: 'Current Stream' as const,
    title: `Open ${stream.user_name} on ${link.title || link.name}`,
    subtitle: link.url.replace(/^https?:\/\/(www\.)?/, ''),
    keywords: `social ${link.name} ${link.title} ${stream.user_name} ${link.url}`.toLowerCase(),
    initial: socialInitial(link),
    run: async () => {
      try {
        await invoke('open_browser_url', { url: link.url });
      } catch (e) {
        Logger.warn('[CommandPalette] social link open failed:', e);
      }
    },
  }));
}

// ---------- Recent chatters -------------------------------------------------

export function getRecentChatterItems(): PaletteItem[] {
  const users = Array.from(useChatUserStore.getState().users.values());
  users.sort((a, b) => b.lastSeen - a.lastSeen);
  return users.slice(0, 25).map((u) => ({
    id: `chatter.${u.userId}`,
    section: 'Recent Chatters' as const,
    title: u.displayName || u.username,
    subtitle: `@${u.username} · seen in current chat`,
    initial: (u.username || '?').slice(0, 1).toUpperCase(),
    keywords: `${u.username} ${u.displayName}`.toLowerCase(),
    twitchUserId: u.userId,
    run: () => {
      useAppStore.getState().openWhisperWithUser({
        id: u.userId,
        login: u.username,
        display_name: u.displayName,
      });
    },
  }));
}

// ---------- Twitch live search ---------------------------------------------

export async function searchTwitchChannels(query: string): Promise<PaletteItem[]> {
  const q = query.trim();
  if (!q) return [];
  try {
    const results = (await invoke('search_channels', { query: q })) as TwitchStream[];
    return results.slice(0, 10).map((stream) => {
      const parts: string[] = [];
      if (stream.game_name) parts.push(stream.game_name);
      if (typeof stream.viewer_count === 'number' && stream.viewer_count > 0) {
        parts.push(`${stream.viewer_count.toLocaleString()} viewers · LIVE`);
      } else {
        parts.push('Offline');
      }
      return {
        id: `streamer.${stream.user_id || stream.user_login}`,
        section: 'Streamers' as const,
        title: stream.user_name || stream.user_login,
        subtitle: parts.join(' · '),
        avatarUrl: stream.thumbnail_url || stream.profile_image_url || undefined,
        initial: (stream.user_login || stream.user_name || '?').slice(0, 1).toUpperCase(),
        keywords: `${stream.user_login} ${stream.user_name} ${stream.game_name ?? ''}`.toLowerCase(),
        twitchUserId: stream.user_id || undefined,
        run: () => useAppStore.getState().startStream(stream.user_login, stream),
      };
    });
  } catch (err) {
    Logger.warn('[CommandPalette] Twitch search failed:', err);
    return [];
  }
}

// ---------- Twitch category search -----------------------------------------

interface CategoryHit {
  id?: string;
  name?: string;
  box_art_url?: string;
}

/** Debounced category lookup. Each hit expands into TWO palette rows: one to
 *  browse the Home category page for that game, and one to open Drops with
 *  the game name pre-filled. Cap at 3 categories (so 6 rows max) to keep
 *  the section scannable when both streamers and categories return results. */
export async function searchTwitchCategories(query: string): Promise<PaletteItem[]> {
  const q = query.trim();
  if (!q) return [];
  try {
    const raw = (await invoke('search_categories', { query: q, limit: 5 })) as unknown;
    const hits: CategoryHit[] = parseCategoryHits(raw);
    const items: PaletteItem[] = [];
    for (const hit of hits.slice(0, 3)) {
      const name = hit.name?.trim();
      if (!name) continue;
      const initial = name.slice(0, 1).toUpperCase();
      const box = hit.box_art_url ? hit.box_art_url.replace('{width}', '52').replace('{height}', '72') : undefined;
      items.push(
        {
          id: `category.browse.${name}`,
          section: 'Categories',
          title: `Browse ${name}`,
          subtitle: 'Open the Home category page',
          avatarUrl: box,
          initial,
          keywords: `category browse ${name.toLowerCase()}`,
          run: () => useAppStore.getState().navigateToCategoryByName(name),
        },
        {
          id: `category.drops.${name}`,
          section: 'Categories',
          title: `View drops for ${name}`,
          subtitle: 'Open Drops, filtered to this game',
          avatarUrl: box,
          initial,
          keywords: `category drops ${name.toLowerCase()} campaigns`,
          run: () => useAppStore.getState().openDropsWithSearch(name),
        },
      );
    }
    return items;
  } catch (err) {
    Logger.warn('[CommandPalette] Twitch category search failed:', err);
    return [];
  }
}

function parseCategoryHits(raw: unknown): CategoryHit[] {
  if (!raw || typeof raw !== 'object') return [];
  // Twitch Helix returns `{ data: [...], pagination: {...} }`. Our Rust
  // wrapper exposes the raw json, so unwrap conservatively in case the shape
  // changes (e.g. service starts returning just `data`).
  const maybe = raw as { data?: unknown };
  const data = Array.isArray(maybe.data) ? maybe.data : Array.isArray(raw as unknown[]) ? (raw as unknown[]) : [];
  return data.filter((d): d is CategoryHit => !!d && typeof d === 'object').map((d) => {
    const obj = d as Record<string, unknown>;
    return {
      id: typeof obj.id === 'string' ? obj.id : undefined,
      name: typeof obj.name === 'string' ? obj.name : undefined,
      box_art_url: typeof obj.box_art_url === 'string' ? obj.box_art_url : undefined,
    };
  });
}

// ---------- Streamer enrichment (lazy about-data fetch) --------------------

/** Resolved ChannelAboutData (bio + socials + panels + follower count + game)
 *  keyed by twitch user_id. Persists for the life of the window/process — the
 *  data changes infrequently and re-fetching on every palette open would be
 *  wasteful. Stores the WHOLE payload, not just description, because the
 *  social-link rows for the current stream also pull from here. */
const aboutCache = new Map<string, ChannelAboutData>();
/** Negative cache: ids we've asked about and got nothing useful back from.
 *  Distinguishes "asked, returned empty" from "not yet asked". */
const aboutFetched = new Set<string>();

/** Lazy-fetch the streamer's about-data the first time it's requested for a
 *  given user_id; cached forever after. Returns the description specifically
 *  so existing callers (the active-row enrichment in CommandPalette.tsx) keep
 *  working with the same shape. */
export async function enrichStreamerDescription(userId: string, login: string): Promise<string | null> {
  await ensureAboutFetched(userId, login);
  return aboutCache.get(userId)?.description?.trim() || null;
}

async function ensureAboutFetched(userId: string, login: string): Promise<void> {
  if (aboutCache.has(userId) || aboutFetched.has(userId)) return;
  aboutFetched.add(userId);
  try {
    const payload = (await invoke('get_channel_about_data', { channelLogin: login })) as ChannelAboutData;
    if (payload) aboutCache.set(userId, payload);
  } catch (err) {
    Logger.warn('[CommandPalette] get_channel_about_data failed:', err);
  }
}

/** Backwards-compatible accessor for the description-only case (still used by
 *  the active-row preview in CommandPalette.tsx). */
export function getCachedDescription(userId: string): string | undefined {
  return aboutCache.get(userId)?.description?.trim() || undefined;
}

export function getCachedAbout(userId: string): ChannelAboutData | undefined {
  return aboutCache.get(userId);
}

/** Eager warmup — called from CommandPalette.tsx when the currentStream
 *  changes so the social-link rows are ready before the user opens the
 *  palette. Safe to call repeatedly; the dedup is via `aboutFetched`. */
export function warmupAbout(userId: string, login: string): void {
  void ensureAboutFetched(userId, login);
}

// ---------- Matcher ---------------------------------------------------------

export function scoreItem(item: PaletteItem, queryLower: string): number {
  if (!queryLower) return 0;

  // Alias matches win, hard. A user explicitly bound this snippet to this
  // keystroke; we trust that intent over any title-text fuzz.
  if (item.alias) {
    if (item.alias === queryLower) return 1500;
    if (item.alias.startsWith(queryLower)) return 1200 - (item.alias.length - queryLower.length);
  }

  const title = item.title.toLowerCase();
  const subtitle = (item.subtitle ?? '').toLowerCase();
  const keywords = item.keywords ?? '';

  if (title === queryLower) return 1000;
  if (title.startsWith(queryLower)) return 700 - (title.length - queryLower.length);
  const titleIdx = title.indexOf(queryLower);
  if (titleIdx !== -1) return 500 - titleIdx;
  if (subtitle.includes(queryLower)) return 300;
  if (keywords.includes(queryLower)) return 200;

  const haystack = `${title} ${subtitle} ${keywords}`;
  let lastIdx = -1;
  let gaps = 0;
  for (const ch of queryLower) {
    const nextIdx = haystack.indexOf(ch, lastIdx + 1);
    if (nextIdx === -1) return -1;
    if (lastIdx !== -1) gaps += nextIdx - lastIdx - 1;
    lastIdx = nextIdx;
  }
  return 100 - Math.min(gaps, 80);
}

// ---------- Recent-commands persistence ------------------------------------

const RECENT_KEY = 'streamnook.commandPalette.recent.v1';
const RECENT_MAX = 6;

export function loadRecentCommandIds(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (Array.isArray(parsed)) return parsed.filter((x): x is string => typeof x === 'string').slice(0, RECENT_MAX);
    return [];
  } catch {
    return [];
  }
}

export function pushRecentCommand(id: string): void {
  try {
    const current = loadRecentCommandIds().filter((x) => x !== id);
    current.unshift(id);
    localStorage.setItem(RECENT_KEY, JSON.stringify(current.slice(0, RECENT_MAX)));
  } catch {
    // localStorage can throw under private mode / quota; nice-to-have only.
  }
}

