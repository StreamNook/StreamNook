// The page actions a slash command can ask for (`ui` in the answer of
// `run_slash_command`, services/slash_dispatch.rs). Rust runs every command;
// these are the only parts that need this page: its video element, its
// windows, its own chat buffer and stores, the speech engine, and the
// whole-window effects. Nothing here decides what a command does.

import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { injectSystemMessage, systemSourceFor } from '../stores/chatConnectionStore';
import { Logger } from './logger';

export type SlashUi =
  | { kind: 'open_user_card'; user_id: string; login: string; display_name: string }
  | { kind: 'clear_local_chat' }
  | { kind: 'open_poll_composer'; channel: string; composer: 'poll' | 'prediction' }
  | { kind: 'create_clip'; channel: string; channel_id: string; title: string }
  | { kind: 'identify_song' }
  | { kind: 'reload_stream' }
  | { kind: 'reload_chat' }
  | { kind: 'refresh_emotes' }
  | { kind: 'open_chat_window'; window: 'overlay' | 'multichat'; channel: string; channel_id: string | null }
  | { kind: 'open_channel_points'; channel: string }
  | { kind: 'open_search' }
  | { kind: 'local_view'; view: 'mod' | 'sub' | 'ascii'; on: boolean }
  | { kind: 'dock_join'; login: string; channel_id: string; display_name: string; avatar_url: string | null }
  | { kind: 'dock_part'; login: string }
  | { kind: 'speak'; text: string }
  | { kind: 'window_effect'; effect: 'barrelroll' | 'party' }
  | { kind: 'screenshot' };

const toast = (msg: string, kind: 'success' | 'error' | 'info' = 'info') => useAppStore.getState().addToast(msg, kind);

/** A line in the chat the command was typed in (only you see it). */
export function noticeIn(channel: string, message: string): void {
  if (channel) {
    // Twitch keys are lowercase logins; other platforms' composite keys keep
    // their case (YouTube channel ids are case sensitive).
    const key = channel.includes(':') ? channel : channel.toLowerCase();
    injectSystemMessage(key, message, undefined, systemSourceFor(key));
    return;
  }
  window.dispatchEvent(new CustomEvent('twitch-system-message', { detail: { message } }));
}

/** A short whole-window animation for /barrelroll and /party. Transform and
 *  opacity only (compositor work), and skipped under reduced motion. */
function playWindowEffect(kind: 'barrelroll' | 'party'): boolean {
  const reduced =
    document.documentElement.dataset.motion === 'reduced' ||
    window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  if (reduced) return false;
  if (kind === 'barrelroll') {
    const root = document.getElementById('root') ?? document.body;
    root.animate([{ transform: 'rotate(0deg)' }, { transform: 'rotate(360deg)' }], {
      duration: 2000,
      easing: 'ease-in-out',
    });
    return true;
  }
  const layer = document.createElement('div');
  layer.setAttribute('aria-hidden', 'true');
  Object.assign(layer.style, {
    position: 'fixed',
    inset: '0',
    pointerEvents: 'none',
    zIndex: '2147483646',
    mixBlendMode: 'overlay',
    background: 'linear-gradient(120deg, #ff3b6b, #ffb800, #3bff8a, #3bb8ff, #b83bff)',
    opacity: '0',
  });
  document.body.appendChild(layer);
  const anim = layer.animate([{ opacity: 0 }, { opacity: 0.55 }, { opacity: 0 }], {
    duration: 1000,
    iterations: 4,
  });
  anim.onfinish = () => layer.remove();
  return true;
}

function findPlayerVideo(): HTMLVideoElement | null {
  const videos = Array.from(document.querySelectorAll('video'));
  return (
    videos
      .filter((v) => v.offsetWidth > 0 && v.offsetHeight > 0)
      .sort((a, b) => b.offsetWidth * b.offsetHeight - a.offsetWidth * a.offsetHeight)[0] ?? null
  );
}

async function screenRectOf(el: HTMLElement) {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  const win = getCurrentWindow();
  const [pos, scale] = await Promise.all([win.innerPosition(), win.scaleFactor()]);
  const r = el.getBoundingClientRect();
  const x = Math.floor(pos.x + r.left * scale);
  const y = Math.floor(pos.y + r.top * scale);
  return {
    x,
    y,
    width: Math.max(1, Math.ceil(pos.x + r.right * scale) - x),
    height: Math.max(1, Math.ceil(pos.y + r.bottom * scale) - y),
  };
}

/** Do what `ui` asks in the room `channel` (Twitch id `channelId`). Kinds that
 *  only start something (a clip, song recognition, a stream reload) return
 *  before it finishes. */
export async function applySlashUi(ui: SlashUi, channel: string, channelId: string): Promise<void> {
  try {
    switch (ui.kind) {
      case 'open_user_card':
        // The active pane (main chat or a MultiChat window) decides where.
        window.dispatchEvent(
          new CustomEvent('streamnook-open-user-card', {
            detail: { userId: ui.user_id, username: ui.login, displayName: ui.display_name },
          }),
        );
        return;
      case 'clear_local_chat':
        window.dispatchEvent(new CustomEvent('streamnook-clear-local-chat'));
        return;
      case 'open_poll_composer':
        window.dispatchEvent(
          new CustomEvent('streamnook-open-poll-composer', { detail: { channel: ui.channel, kind: ui.composer } }),
        );
        return;
      case 'create_clip':
        void useAppStore
          .getState()
          .createClip()
          .catch((err) => {
            Logger.error('[Slash] /clip failed:', err);
            toast('Could not create a clip', 'error');
          });
        return;
      case 'identify_song': {
        const { recognizeNowPlaying, announceSong } = await import('./songId');
        void recognizeNowPlaying().then(announceSong);
        return;
      }
      case 'reload_stream': {
        const app = useAppStore.getState();
        if (!app.currentStream?.user_login || app.currentMediaType !== 'live') {
          toast('/reload: no active live stream', 'error');
          return;
        }
        toast('Reloading stream and chat...', 'info');
        void app.reloadStreamAndChat();
        return;
      }
      case 'reload_chat': {
        const { hardRefreshChannel } = await import('../stores/chatConnectionStore');
        await hardRefreshChannel(channel, channelId || null);
        return;
      }
      case 'refresh_emotes': {
        const { refreshChannelEmotes } = await import('../stores/chatConnectionStore');
        if (channelId) void refreshChannelEmotes(channel, channelId);
        return;
      }
      case 'open_chat_window':
        if (ui.window === 'overlay') {
          const { openChatOverlayWindow } = await import('./chatOverlayWindow');
          await openChatOverlayWindow({ channel: ui.channel, channelId: ui.channel_id ?? undefined }).catch((err) => {
            Logger.error('[Slash] /overlay failed:', err);
            toast('Failed to open chat overlay', 'error');
          });
        } else {
          const { openMultiChatWindow } = await import('./multichatWindow');
          await openMultiChatWindow({ channel: ui.channel, channelId: ui.channel_id ?? undefined, newWindow: true }).catch(
            (err) => {
              Logger.error('[Slash] /popup failed:', err);
              toast('Failed to open MultiChat window', 'error');
            },
          );
        }
        return;
      case 'open_channel_points':
        window.dispatchEvent(new CustomEvent('streamnook-open-channel-points', { detail: { channel: ui.channel } }));
        return;
      case 'open_search': {
        const { getChatSearchController } = await import('../keybindings/chatSearchController');
        const controller = getChatSearchController();
        if (!controller) toast('Chat search is not available here', 'info');
        else controller.openSearch();
        return;
      }
      case 'local_view': {
        const { useLocalChatViewStore } = await import('../stores/localChatViewStore');
        useLocalChatViewStore.getState().setView(channel, ui.view, ui.on);
        return;
      }
      case 'dock_join': {
        const { dockChat, setDockPanelOpen } = await import('../stores/chatDockStore');
        await dockChat(
          {
            provider: 'twitch',
            login: ui.login,
            channel_id: ui.channel_id,
            display_name: ui.display_name,
            avatar_url: ui.avatar_url,
          },
          true,
        );
        setDockPanelOpen(true);
        return;
      }
      case 'dock_part': {
        const { undockChat, useChatDockStore } = await import('../stores/chatDockStore');
        const docked = useChatDockStore
          .getState()
          .chats.find((c) => c.provider === 'twitch' && c.login.toLowerCase() === ui.login);
        if (!docked) {
          toast(`${ui.login || 'That chat'} is not in your chat dock`, 'info');
          return;
        }
        await undockChat(`twitch:${docked.login.toLowerCase()}`);
        toast(`Left ${docked.display_name || docked.login}'s chat`, 'success');
        return;
      }
      case 'speak': {
        const synth = typeof window !== 'undefined' ? window.speechSynthesis : undefined;
        if (!synth) toast('Text to speech is not available on this system', 'error');
        else synth.speak(new SpeechSynthesisUtterance(ui.text));
        return;
      }
      case 'window_effect':
        if (!playWindowEffect(ui.effect)) toast('Skipped: reduced motion is on', 'info');
        return;
      case 'screenshot': {
        const video = findPlayerVideo();
        if (!video) {
          toast('No stream is playing to take a screenshot of', 'error');
          return;
        }
        const rect = await screenRectOf(video);
        const path = await invoke<string>('save_stream_screenshot', { ...rect, channel: channel || null });
        noticeIn(channel, `Screenshot saved: ${path}`);
        return;
      }
    }
  } catch (err) {
    Logger.error(`[Slash] ${ui.kind} failed:`, err);
    toast(err instanceof Error ? err.message : String(err), 'error');
  }
}
