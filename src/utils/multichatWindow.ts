// Helper for spawning a StreamNook MultiChat popout window. Uses Tauri's
// WebviewWindow with the same index.html as the main app, routed via the
// `#/multichat` hash so main.tsx renders the MultiChatWindow shell instead
// of the regular App.
//
// Single-popout model: the popout is a singleton keyed by `WINDOW_LABEL`. If
// one already exists, a second `openMultiChatWindow` call focuses it (and,
// when a channel is provided, emits `multichat-add-channel` so the existing
// window appends the new channel as a tab). Storage uses a stable id
// (`WINDOW_ID`) so closing and reopening restores the same tab set instead
// of leaving an orphan localStorage record per session.

import { Logger } from './logger';

export interface OpenMultiChatOptions {
  /** Optional channel to pre-load (used when popping out from a watched stream).
   *  If omitted, the window opens empty for the user to add channels manually. */
  channel?: string;
  /** Twitch channel/room id, paired with `channel`. Without this the optimistic
   *  IRC send path can't supply a real `room-id` tag, and channel-scoped badges
   *  fall through to global until USERSTATE lands. */
  channelId?: string;
  /** Display name (proper capitalization) for the channel — used for the tab
   *  label and window title until the popout's own metadata poll lands. */
  channelName?: string;
  /** Multiple channels to seed/add at once — e.g. popping out every MultiNook
   *  tile's chat in one click. Takes precedence over the single-channel fields. */
  channels?: Array<{ channel: string; channelId?: string | null; channelName?: string | null }>;
  /** Replace the popout's entire tab set with exactly these channels (a fresh
   *  view) instead of merging/appending into whatever was already open. */
  replace?: boolean;
  /** Open a SEPARATE MultiChat window instead of focusing the default one.
   *  Each window is its own WebView2 renderer (roughly 150 MB), so this is a
   *  deliberate user action, never an implicit fallback. */
  newWindow?: boolean;
  /** Display title (defaults to `StreamNook MultiChat` or includes the channel
   *  name when one is pre-loaded). */
  title?: string;
}

// Defaults tuned to feel comparable to Twitch's stock popout chat window.
// Twitch's web popout is roughly 340×500 of chat-only content; we add ~70px
// for our own chrome (custom title bar + tab strip + send input row), which
// lands us around 402×620 — compact, comfortable on a second monitor, and
// resizable from any edge if the user wants more room.
// Width of a single chat column. Split layouts (2–4 columns) size the window to
// a multiple of this so each chat keeps a comfortable single-chat width instead
// of being squeezed into a fraction of one. Exported so MultiChatWindow's
// column-aware resize uses the exact same base. Rust opens new windows at this
// width (commands/popout_window.rs); keep the two in step.
export const MULTICHAT_BASE_WIDTH = 402;

const WINDOW_ID = 'default';
const WINDOW_LABEL = `multichat-${WINDOW_ID}`;
const STORAGE_PREFIX = 'streamnook.multichat.';
const KEEP_STORAGE_KEY = `${STORAGE_PREFIX}${WINDOW_ID}`;
/** Extra windows are `multichat-w2`, `multichat-w3`, ... (first free id). */
const EXTRA_ID_PREFIX = 'w';
const MAX_EXTRA_WINDOWS = 8;

/** Sweep orphan `streamnook.multichat.<id>` keys: the pre-stable-id random
 *  ids, and extra windows (`w2`, `w3`, ...) that are no longer open. The
 *  default window's state is always kept. Cheap to run on every spawn. */
function cleanupOrphanStorage(openIds: Set<string>): void {
  try {
    const orphans: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (!key || !key.startsWith(STORAGE_PREFIX) || key === KEEP_STORAGE_KEY) continue;
      const id = key.slice(STORAGE_PREFIX.length);
      if (openIds.has(id)) continue;
      orphans.push(key);
    }
    for (const key of orphans) localStorage.removeItem(key);
    if (orphans.length > 0) {
      Logger.debug(`[MultiChat] Cleaned ${orphans.length} orphan storage key(s)`);
    }
  } catch (err) {
    Logger.warn('[MultiChat] orphan storage sweep failed:', err);
  }
}

export async function openMultiChatWindow(options: OpenMultiChatOptions = {}): Promise<void> {
  try {
    const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
    const { invoke } = await import('@tauri-apps/api/core');
    const { emit } = await import('@tauri-apps/api/event');

    // Which MultiChat windows exist right now (labels `multichat-<id>`).
    const openIds = new Set<string>();
    try {
      for (const w of await WebviewWindow.getAll()) {
        if (w.label.startsWith('multichat-')) openIds.add(w.label.slice('multichat-'.length));
      }
    } catch (err) {
      Logger.debug('[MultiChat] getAll failed:', err);
    }
    cleanupOrphanStorage(openIds);

    // Target window: the default (focus + add if it exists) or the first free
    // extra id when the caller asked for a separate window.
    let windowId = WINDOW_ID;
    if (options.newWindow) {
      let n = 2;
      while (openIds.has(`${EXTRA_ID_PREFIX}${n}`) && n <= MAX_EXTRA_WINDOWS + 1) n += 1;
      if (n > MAX_EXTRA_WINDOWS + 1) {
        Logger.warn(`[MultiChat] ${MAX_EXTRA_WINDOWS} extra windows already open`);
        return;
      }
      windowId = `${EXTRA_ID_PREFIX}${n}`;
    }
    const windowLabel = `multichat-${windowId}`;
    const isExtra = windowId !== WINDOW_ID;

    // Normalize to a single channel list, lowercased. `channels` (multi) wins
    // over the single-channel fields; either way the rest of the flow is uniform.
    const channelList = (options.channels && options.channels.length > 0
      ? options.channels
      : options.channel
        ? [{ channel: options.channel, channelId: options.channelId, channelName: options.channelName }]
        : []
    ).map((c) => ({
      channel: c.channel.toLowerCase(),
      channelId: c.channelId ?? null,
      channelName: c.channelName ?? c.channel,
    }));

    // If a popout already exists, focus it and ask it to add each requested
    // channel as a tab. The popout listens for `multichat-add-channel` and
    // routes each through its add/dedup path, so channels already open are no-ops.
    const existing = isExtra ? null : await WebviewWindow.getByLabel(WINDOW_LABEL);
    if (existing) {
      try {
        if (await existing.isMinimized()) await existing.unminimize();
        await existing.show();
        await existing.setFocus();
      } catch (err) {
        Logger.warn('[MultiChat] focus existing popout failed:', err);
      }
      if (options.replace) {
        // Replace the popout's whole tab set with exactly this list.
        try {
          await emit('multichat-set-channels', { channels: channelList });
        } catch (err) {
          Logger.warn('[MultiChat] emit multichat-set-channels failed:', err);
        }
      } else {
        for (const c of channelList) {
          try {
            await emit('multichat-add-channel', {
              channel: c.channel,
              channelId: c.channelId,
              channelName: c.channelName,
            });
          } catch (err) {
            Logger.warn('[MultiChat] emit multichat-add-channel failed:', err);
          }
        }
      }
      return;
    }

    const title =
      options.title ??
      (channelList.length > 1
        ? `StreamNook MultiChat — ${channelList.length} channels`
        : channelList.length === 1
          ? `StreamNook MultiChat — ${channelList[0].channelName}`
          : 'StreamNook MultiChat');

    // Rust builds and places the window (commands/popout_window.rs): the spot
    // this window had last time, or beside this one, always wholly on screen.
    // The seeded channels ride its URL, so a new window never races a listener
    // that is not up yet.
    await invoke('open_multichat_window', {
      id: windowId,
      channels: channelList,
      replace: !!options.replace,
      title,
    });

    Logger.debug(`[MultiChat] Opened window ${windowLabel} for channel ${options.channel ?? '(empty)'}`);
  } catch (err) {
    Logger.error('[MultiChat] openMultiChatWindow failed:', err);
    throw err;
  }
}

// Expose on window during development so the popout can be triggered from
// devtools while the UI button is still being designed. Safe in production
// since the function only spawns a known-label window with our own origin.
if (typeof window !== 'undefined') {
  (window as unknown as { openMultiChatWindow: typeof openMultiChatWindow }).openMultiChatWindow =
    openMultiChatWindow;
}
