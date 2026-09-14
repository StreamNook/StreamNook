import { invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';
import { historyKey } from './chatterIdentity';

/**
 * Open the cursor-anchored user-profile popout window (the `/#/profile`
 * WebviewWindow). This is THE implementation — chat, the mod log and MultiChat all
 * route through it, because the placement math below (physical/logical units and
 * the per-monitor clamp) is easy to get subtly wrong in a second copy.
 *
 * Returns false when the window could not be opened, so a caller can fall back to
 * the in-app modal.
 */
export async function openProfilePopup(opts: {
  userId: string;
  username: string; // login
  displayName?: string;
  color?: string;
  badges?: Array<{ key: string; info: unknown }>;
  channelId?: string;
  channelName?: string;
  /** Whether the viewer is a moderator/broadcaster in this channel. Drives the
   *  card's Moderator Actions zone (it also needs the resolved channel id as the
   *  broadcaster target, which the page derives from `channelId`). */
  isModerator?: boolean;
  /** Which platform this chatter is on. Absent means Twitch. */
  provider?: import('../types/providers').ProviderId;
  clientX?: number;
  clientY?: number;
}): Promise<boolean> {
  try {
    const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
    const { getCurrentWindow, availableMonitors, currentMonitor, PhysicalPosition } = await import(
      '@tauri-apps/api/window'
    );
    const mainWindow = getCurrentWindow();
    // innerPosition (the client-area top-left) plus the scale factor put the
    // cursor, whose clientX/Y are logical CSS pixels inside this window, into
    // physical desktop coordinates. That is the same space monitor bounds are
    // reported in, so the clamp math below stays consistent. outerPosition
    // would be off by the invisible DWM frame margin Windows reserves for the
    // drop shadow.
    const [winPos, scale] = await Promise.all([
      mainWindow.innerPosition(),
      mainWindow.scaleFactor(),
    ]);

    // Logical card size. These MUST match the width/height we request for the
    // window below, since the clamp converts them to physical via the scale.
    const cardWidth = 402;
    const cardHeight = 680;
    const gap = 10;

    // Cursor position in physical desktop pixels (falls back to this window's
    // origin if no click coords were supplied).
    const cursorX = winPos.x + (opts.clientX ?? 0) * scale;
    const cursorY = winPos.y + (opts.clientY ?? 0) * scale;

    // Card footprint and spacing in physical pixels, for clamping.
    const cardW = cardWidth * scale;
    const cardH = cardHeight * scale;
    const margin = 8 * scale;
    const gapPx = gap * scale;

    // Find the monitor the cursor sits on so the popup stays on-screen even on a
    // multi-monitor setup (a click on a secondary display should anchor there).
    // Fall back to the window's monitor, then to no clamp if neither resolves.
    let monitor: Awaited<ReturnType<typeof currentMonitor>> = null;
    try {
      const monitors = await availableMonitors();
      monitor =
        monitors.find(
          (m) =>
            cursorX >= m.position.x &&
            cursorX < m.position.x + m.size.width &&
            cursorY >= m.position.y &&
            cursorY < m.position.y + m.size.height,
        ) ?? null;
    } catch {
      /* availableMonitors unsupported on this platform, fall through */
    }
    if (!monitor) {
      try {
        monitor = await currentMonitor();
      } catch {
        /* ignore, clamp is skipped below */
      }
    }

    // Anchor to the LEFT of the cursor, vertically centered (the chat-click
    // behavior), then clamp the whole card inside the monitor so opening from a
    // log row near a screen edge can't push half the card off-screen.
    let x = cursorX - cardW - gapPx;
    let y = cursorY - cardH / 2;
    if (monitor) {
      const left = monitor.position.x;
      const top = monitor.position.y;
      const right = monitor.position.x + monitor.size.width;
      const bottom = monitor.position.y + monitor.size.height;
      // Not enough room on the left? Flip to the right of the cursor.
      if (x < left + margin) x = cursorX + gapPx;
      // Keep the full card within the monitor on both axes.
      x = Math.min(x, right - cardW - margin);
      x = Math.max(x, left + margin);
      y = Math.min(y, bottom - cardH - margin);
      y = Math.max(y, top + margin);
    } else {
      if (x < 0) x = cursorX + gapPx;
      if (y < 0) y = 0;
    }

    // NOT passed as the window's x/y options. Those are LOGICAL pixels, and Tauri
    // has to pick a scale factor to convert them before the window exists — which
    // is not necessarily the scale of the monitor we just clamped to. On a
    // mixed-DPI desktop that inflates the coordinate and throws the card onto the
    // next monitor. Physical coordinates are unambiguous, so the window is created
    // hidden and moved with an explicit PhysicalPosition below.
    const physX = Math.round(x);
    const physY = Math.round(y);

    let userId = opts.userId;
    let displayName = opts.displayName || opts.username;
    let channelId = opts.channelId || '';

    // The profile page fetches by user id; if we only have a login (e.g. an IRC
    // CLEARMSG delete carries no id), resolve it first so the popup still works.
    // Likewise resolve the CHANNEL id from its login when the caller supplied a
    // channel name but no id (the mod log only carries a channel login). Badges
    // are channel-scoped: with no channel the profile page falls back to the
    // viewed user's OWN channel, where everyone reads as the broadcaster. Run
    // both lookups together so we don't stack two round-trips.
    //
    // Twitch only. `get_user_by_login` is Helix, so asking it about a Kick slug or
    // a YouTube UC id returns a same-named Twitch account or nothing — either way
    // the wrong person. Non-Twitch ids arrive already canonical from the message,
    // so there is nothing to resolve.
    const isTwitch = (opts.provider ?? 'twitch') === 'twitch';
    const needUser = isTwitch && !userId && !!opts.username;
    const needChannel = isTwitch && !channelId && !!opts.channelName;
    if (needUser || needChannel) {
      const [userInfo, channelInfo] = await Promise.all([
        needUser
          ? invoke<{ id?: string; display_name?: string }>('get_user_by_login', {
              login: opts.username.toLowerCase(),
            }).catch(() => null)
          : Promise.resolve(null),
        needChannel
          ? invoke<{ id?: string }>('get_user_by_login', {
              login: opts.channelName!.toLowerCase(),
            }).catch(() => null)
          : Promise.resolve(null),
      ]);
      if (userInfo?.id) userId = userInfo.id;
      if (needUser && !opts.displayName && userInfo?.display_name) {
        displayName = userInfo.display_name;
      }
      if (channelInfo?.id) channelId = channelInfo.id;
    }

    const windowLabel = `profile-${userId || opts.username}-${Date.now()}`;

    // Seed only. This rides in the popout's URL, so it must stay small: the
    // backing store keeps up to 200 messages per user and serializing all of
    // them would put tens of KB into a query string. The profile page polls
    // `get_user_message_history` for the full set once it mounts, so the seed
    // exists purely to avoid an empty first paint.
    const HISTORY_SEED_LIMIT = 40;
    let messageHistory: unknown[] = [];
    if (userId) {
      try {
        messageHistory = await invoke<unknown[]>('get_user_message_history_limited', {
          // Namespaced for non-Twitch platforms, matching how the message was
          // stored; the bare id would read a different (or empty) bucket.
          userId: historyKey(userId, opts.provider ?? 'twitch'),
          limit: HISTORY_SEED_LIMIT,
        });
      } catch {
        /* no history is fine */
      }
    }

    const params = new URLSearchParams({
      userId: userId || '',
      username: opts.username || '',
      displayName,
      color: opts.color || '',
      badges: JSON.stringify(opts.badges || []),
      channelId: channelId || '',
      channelName: opts.channelName || '',
      isModerator: opts.isModerator ? '1' : '',
      provider: opts.provider ?? 'twitch',
      messageHistory: JSON.stringify(messageHistory),
    });

    const profileWindow = new WebviewWindow(windowLabel, {
      url: `${window.location.origin}/#/profile?${params.toString()}`,
      title: `${displayName}'s Profile`,
      width: cardWidth,
      height: cardHeight,
      resizable: false,
      decorations: false,
      alwaysOnTop: true,
      skipTaskbar: true,
      transparent: true,
      // Created hidden and shown after it is positioned, so the corrective move
      // can't flash the card at the wrong spot (or on the wrong monitor).
      visible: false,
      focus: false,
    });

    const created = await new Promise<boolean>((resolve) => {
      let settled = false;
      const done = (ok: boolean) => {
        if (settled) return;
        settled = true;
        resolve(ok);
      };
      void profileWindow.once('tauri://created', () => done(true));
      void profileWindow.once('tauri://error', (e) => {
        Logger.error('Error opening profile window:', e);
        done(false);
      });
      // A window that never reports either way still gets shown rather than
      // stranded invisible.
      setTimeout(() => done(true), 3000);
    });
    if (!created) return false;

    try {
      await profileWindow.setPosition(new PhysicalPosition(physX, physY));
    } catch (e) {
      Logger.warn('Could not position profile popup, showing at default:', e);
    }
    await profileWindow.show();
    await profileWindow.setFocus().catch(() => { /* focus is best-effort */ });
    return true;
  } catch (err) {
    Logger.error('Failed to open profile popup:', err);
    return false;
  }
}
