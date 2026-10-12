import { invoke } from '@tauri-apps/api/core';
import { ensureLastWhisperListener, getLastWhisperFrom } from './lastWhisper';
import { open as openExternalUrl } from '@tauri-apps/plugin-shell';
import { useAppStore } from '../stores/AppStore';
import { usePinStore } from '../stores/pinStore';
import { Logger } from './logger';
import {
  customCommandWins,
  expandUserCommand,
  findCommandDefinition,
  findUserCommand,
  formatStreamUptime,
  resolveCommandAlias,
  type TemplateContext,
} from './chatCommands';
import { cleanLogin, noticeIn, parseDelay, runExtendedCommand, sendPaced } from './extendedCommands';
import { watchBurstNotices } from './burstStatus';
import { IS_MOBILE } from './platform';
import { parseNukeArgs, executeNuke, executeUndo, isUserModeratorOf } from './nukeEngine';
import { parseRemindCommand, fireReminderNow, formatReminderList, remindHelpLines } from './reminderEngine';
import { recognizeNowPlaying, announceSong } from './songId';

// Build a TemplateContext from the current AppStore + supplied args. Centralized
// so plain-text expansions (ChatWidget) and slash-command expansions (this
// file) populate the same set of placeholders.
export function buildTemplateContext(
  args: string[],
  broadcasterId: string,
  broadcasterLogin: string,
): TemplateContext {
  const state = useAppStore.getState();
  const currentUser = state.currentUser;
  const currentStream = state.currentStream;
  return {
    user_name: currentUser?.display_name || currentUser?.username || currentUser?.login || '',
    user_id: currentUser?.user_id || '',
    channel_name: currentStream?.user_name || currentStream?.user_login || broadcasterLogin || '',
    channel_id: currentStream?.user_id || broadcasterId || '',
    stream_title: currentStream?.title || '',
    stream_game: currentStream?.game_name || '',
    stream_uptime: formatStreamUptime(currentStream?.started_at),
    args,
  };
}

// Open a URL in the OS default browser. Used by /openurl, /popout, and a few
// places that don't want the in-app WebView.
async function openInBrowser(url: string): Promise<void> {
  await openExternalUrl(url);
}

// Emit a synthetic system message into chat (no IRC send). Surface used by
// /uptime, /chatters, /user, /mods, /vips, etc.
function emitSystemMessage(message: string): void {
  window.dispatchEvent(new CustomEvent('twitch-system-message', { detail: { message } }));
}

interface UserLookupResult {
  id: string;
  login: string;
  display_name: string;
}

/**
 * Look up a Twitch user by login name.
 * The Rust command `get_user_by_login` takes a single login string and returns a single UserInfo.
 */
async function lookupUser(username: string): Promise<UserLookupResult | null> {
  try {
    return await invoke<UserLookupResult>('get_user_by_login', { login: username });
  } catch {
    return null;
  }
}

export const handleSlashCommand = async (
  message: string,
  broadcasterId: string,
  broadcasterLogin: string,
  sendMessageToChat: (msg: string) => void,
  // How deep inside /chain, /foreach, /timer or /runin this run is.
  depth = 0,
): Promise<boolean> => {
  if (!message.startsWith('/')) return false;

  const args = message.slice(1).trim().split(/\s+/);
  // What was typed, and the command it runs (`/b` runs `/ban`).
  const typed = args[0].toLowerCase();
  const command = resolveCommandAlias(typed);
  const argsWithoutCommand = args.slice(1);

  const addToast = useAppStore.getState().addToast;

  // User-defined slash commands. The built-ins custom commands have always
  // yielded to win; for every newer built-in name or alias a saved custom
  // command of the same name runs instead (customCommandWins). Plain-text
  // (non-slash) user commands are handled by ChatWidget before this function
  // is called.
  if (customCommandWins(typed)) {
    const state = useAppStore.getState();
    const userCommand = findUserCommand(typed, state.settings.chat_commands?.user_commands, true);
    if (userCommand) {
      const ctx = buildTemplateContext(argsWithoutCommand, broadcasterId, broadcasterLogin);
      const expansion = expandUserCommand(userCommand.expansion, ctx);
      if (expansion.missing_args.length > 0) {
        addToast(
          `/${typed} expects argument${expansion.missing_args.length > 1 ? 's' : ''} ${expansion.missing_args
            .map((i) => `{${i}}`)
            .join(', ')}`,
          'error',
        );
        return true;
      }
      if (!expansion.text) {
        addToast(`/${typed} expanded to an empty message`, 'error');
        return true;
      }
      sendMessageToChat(expansion.text);
      return true;
    }
  }

  const definition = findCommandDefinition(command);
  if (IS_MOBILE && definition?.desktopOnly) {
    addToast(`/${typed} works in the desktop app`, 'info');
    return true;
  }

  try {
    switch (command) {
      // ────────────────────────────────────────────────────────
      // StreamNook-native: identify the music playing in the stream
      // ────────────────────────────────────────────────────────
      // ── /spam and /pyramid: parsed, paced and sent by Rust (run_chat_burst).
      case 'spam':
      case 'pyramid': {
        // Everything after the command word, spacing kept.
        const rawArgs = message.slice(1).trim().slice(command.length);
        try {
          const reply = await invoke<{ message: string; status: unknown }>('run_chat_burst', {
            kind: command,
            args: rawArgs,
            channelLogin: broadcasterLogin,
            channelId: broadcasterId,
          });
          // In the chat it was typed in, whichever chats are open.
          noticeIn(broadcasterLogin, reply.message);
          // The phone has no progress bar above its chat box: how the run
          // ends (done, stopped, failed) arrives as a notice instead.
          if (IS_MOBILE && reply.status) {
            void watchBurstNotices(broadcasterLogin, (line) => noticeIn(broadcasterLogin, line));
          }
        } catch (e) {
          addToast(String(e), 'error');
        }
        return true;
      }
      case 'song': {
        emitSystemMessage('Listening to identify the song...');
        announceSong(await recognizeNowPlaying());
        return true;
      }
      // ────────────────────────────────────────────────────────
      // Moderator / Broadcaster commands (handled via Helix API)
      // ────────────────────────────────────────────────────────
      case 'ban': {
        if (argsWithoutCommand.length >= 1) {
          const username = cleanLogin(argsWithoutCommand[0]);
          const reason = argsWithoutCommand.slice(1).join(' ');
          const user = await lookupUser(username);
          if (user) {
            await invoke('ban_user', { broadcasterId, targetUserId: user.id, duration: null, reason: reason || null });
            addToast(`Banned ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        break;
      }
      case 'timeout': {
        if (argsWithoutCommand.length >= 1) {
          const username = cleanLogin(argsWithoutCommand[0]);
          // "600", "10m", "1h30m", "2d": seconds, or a length with units. When
          // the second word is not a length it starts the reason.
          const lengthMs = parseDelay(argsWithoutCommand[1]);
          const reason = argsWithoutCommand.slice(lengthMs === null ? 1 : 2).join(' ');
          const user = await lookupUser(username);
          if (user) {
            const safeDuration = lengthMs === null ? 600 : Math.max(1, Math.floor(lengthMs / 1000));
            await invoke('ban_user', { broadcasterId, targetUserId: user.id, duration: safeDuration, reason: reason || null });
            addToast(`Timed out ${username} for ${safeDuration}s`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        break;
      }
      case 'unban':
      case 'untimeout': {
        if (argsWithoutCommand.length >= 1) {
          const username = cleanLogin(argsWithoutCommand[0]);
          const user = await lookupUser(username);
          if (user) {
            await invoke('unban_user', { broadcasterId, targetUserId: user.id });
            addToast(`Unbanned ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        break;
      }
      case 'clear': {
        await invoke('clear_chat', { broadcasterId });
        addToast('Chat cleared', 'success');
        return true;
      }
      case 'slow': {
        const slowSeconds = argsWithoutCommand.length > 0 ? parseInt(argsWithoutCommand[0]) : 30;
        const safeSlowSeconds = isNaN(slowSeconds) ? 30 : slowSeconds;
        await invoke('update_chat_settings', { broadcasterId, settings: { slow_mode: true, slow_mode_wait_time: safeSlowSeconds } });
        addToast(`Slow mode enabled (${safeSlowSeconds}s)`, 'success');
        return true;
      }
      case 'slowoff': {
        await invoke('update_chat_settings', { broadcasterId, settings: { slow_mode: false } });
        addToast('Slow mode disabled', 'success');
        return true;
      }
      case 'followers': {
        const durationStr = argsWithoutCommand[0] || '0';
        let followersMinutes = 0;
        if (durationStr.endsWith('mo')) followersMinutes = parseInt(durationStr) * 60 * 24 * 30;
        else if (durationStr.endsWith('m')) followersMinutes = parseInt(durationStr);
        else if (durationStr.endsWith('h')) followersMinutes = parseInt(durationStr) * 60;
        else if (durationStr.endsWith('d')) followersMinutes = parseInt(durationStr) * 60 * 24;
        else if (durationStr.endsWith('w')) followersMinutes = parseInt(durationStr) * 60 * 24 * 7;
        else followersMinutes = parseInt(durationStr) || 0;
        
        await invoke('update_chat_settings', { broadcasterId, settings: { follower_mode: true, follower_mode_duration: followersMinutes } });
        addToast('Followers-only mode enabled', 'success');
        return true;
      }
      case 'followersoff': {
        await invoke('update_chat_settings', { broadcasterId, settings: { follower_mode: false } });
        addToast('Followers-only mode disabled', 'success');
        return true;
      }
      case 'subscribers': {
        await invoke('update_chat_settings', { broadcasterId, settings: { subscriber_mode: true } });
        addToast('Subscribers-only mode enabled', 'success');
        return true;
      }
      case 'subscribersoff': {
        await invoke('update_chat_settings', { broadcasterId, settings: { subscriber_mode: false } });
        addToast('Subscribers-only mode disabled', 'success');
        return true;
      }
      case 'uniquechat': {
        await invoke('update_chat_settings', { broadcasterId, settings: { unique_chat_mode: true } });
        addToast('Unique chat mode enabled', 'success');
        return true;
      }
      case 'uniquechatoff': {
        await invoke('update_chat_settings', { broadcasterId, settings: { unique_chat_mode: false } });
        addToast('Unique chat mode disabled', 'success');
        return true;
      }
      case 'emoteonly': {
        await invoke('update_chat_settings', { broadcasterId, settings: { emote_mode: true } });
        addToast('Emote-only mode enabled', 'success');
        return true;
      }
      case 'emoteonlyoff': {
        await invoke('update_chat_settings', { broadcasterId, settings: { emote_mode: false } });
        addToast('Emote-only mode disabled', 'success');
        return true;
      }
      case 'announce':
      case 'announceblue':
      case 'announcegreen':
      case 'announceorange':
      case 'announcepurple': {
        if (argsWithoutCommand.length > 0) {
          const colorMap: Record<string, string> = {
            announce: 'primary',
            announceblue: 'blue',
            announcegreen: 'green',
            announceorange: 'orange',
            announcepurple: 'purple',
          };
          await invoke('send_chat_announcement', { broadcasterId, message: argsWithoutCommand.join(' '), color: colorMap[command] });
          return true;
        }
        break;
      }
      case 'shoutout': {
        if (argsWithoutCommand.length > 0) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('send_shoutout', { broadcasterId, targetUserId: user.id });
            addToast(`Shoutout given to ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        break;
      }
      case 'commercial': {
        const length = argsWithoutCommand.length > 0 ? parseInt(argsWithoutCommand[0]) : 30;
        const safeLength = isNaN(length) ? 30 : length;
        await invoke('start_commercial', { broadcasterId, length: safeLength });
        addToast(`Started ${safeLength}s commercial`, 'success');
        return true;
      }
      case 'mod': {
        if (argsWithoutCommand.length >= 1) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('add_channel_moderator', { broadcasterId, targetUserId: user.id });
            addToast(`${username} is now a moderator`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        break;
      }
      case 'unmod': {
        if (argsWithoutCommand.length >= 1) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('remove_channel_moderator', { broadcasterId, targetUserId: user.id });
            addToast(`${username} is no longer a moderator`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        break;
      }
      case 'vip': {
        if (argsWithoutCommand.length >= 1) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('add_channel_vip', { broadcasterId, targetUserId: user.id });
            addToast(`${username} is now a VIP`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        break;
      }
      case 'unvip': {
        if (argsWithoutCommand.length >= 1) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('remove_channel_vip', { broadcasterId, targetUserId: user.id });
            addToast(`${username} is no longer a VIP`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        break;
      }
      case 'raid': {
        if (argsWithoutCommand.length >= 1) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('start_raid', { broadcasterId, targetUserId: user.id });
            addToast(`Raiding ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        break;
      }
      case 'unraid': {
        await invoke('cancel_raid', { broadcasterId });
        addToast('Raid cancelled', 'success');
        return true;
      }
      case 'marker': {
        const description = argsWithoutCommand.length > 0 ? argsWithoutCommand.join(' ') : undefined;
        await invoke('create_stream_marker', { userId: broadcasterId, description: description || null });
        addToast('Stream marker created', 'success');
        return true;
      }

      // ────────────────────────────────────────────────────────
      // Everyone commands (handled via Helix API where we have endpoints)
      // ────────────────────────────────────────────────────────
      case 'color': {
        if (argsWithoutCommand.length >= 1) {
          const currentUser = useAppStore.getState().currentUser;
          if (!currentUser?.user_id) {
            addToast('You must be logged in to change your color', 'error');
            return true;
          }
          const colorValue = argsWithoutCommand[0];
          try {
            await invoke('update_user_chat_color', { targetUserId: currentUser.user_id, color: colorValue });
            addToast(`Chat color changed to ${colorValue}`, 'success');
          } catch (err: unknown) {
            const errMsg = err instanceof Error ? err.message : String(err);
            addToast(`Failed to change color: ${errMsg}`, 'error');
          }
          return true;
        }
        // No color provided — show usage instead of passthrough
        addToast('Usage: /color <color name or hex>', 'info');
        return true;
      }
      case 'block': {
        if (argsWithoutCommand.length >= 1) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('block_user', { targetUserId: user.id });
            addToast(`Blocked ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        break;
      }
      case 'unblock': {
        if (argsWithoutCommand.length >= 1) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('unblock_user', { targetUserId: user.id });
            addToast(`Unblocked ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        break;
      }
      case 'w': {
        // /w username message — send whisper
        if (argsWithoutCommand.length >= 2) {
          const username = argsWithoutCommand[0].replace('@', '');
          const whisperMessage = argsWithoutCommand.slice(1).join(' ');
          const user = await lookupUser(username);
          if (user) {
            await invoke('send_whisper', { toUserId: user.id, message: whisperMessage });
            addToast(`Whisper sent to ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        if (argsWithoutCommand.length === 1) {
          addToast('Usage: /w <username> <message>', 'info');
          return true;
        }
        break;
      }
      case 'disconnect': {
        // Disconnect from chat
        try {
          // Rust keeps the connection while another chat window still uses it.
          const disconnected = await invoke<boolean>('stop_chat');
          addToast(
            disconnected
              ? 'Disconnected from chat'
              : 'Chat stayed connected because another chat window is still using it',
            'info',
          );
        } catch (err: unknown) {
          Logger.error('[Command Handler] Disconnect failed:', err);
        }
        return true;
      }

      // ────────────────────────────────────────────────────────
      // IRC passthrough — commands handled natively by Twitch IRC
      // ────────────────────────────────────────────────────────
      case 'mods':
      case 'vips': {
        // Use GQL ChatViewers query — works for ANY authenticated user
        // (Helix Get Moderators requires broadcaster_id == access token user_id)
        const chatters = await invoke<{ moderators?: { login: string }[]; vips?: { login: string }[] }>(
          'get_chatters_by_role', { channelLogin: broadcasterLogin }
        );
        if (command === 'mods') {
          const mods = (chatters.moderators || []).map(m => m.login).filter(Boolean);
          const modMsg = mods.length > 0
            ? `The moderators of this channel are: ${mods.join(', ')}`
            : 'There are no moderators of this channel.';
          window.dispatchEvent(new CustomEvent('twitch-system-message', { detail: { message: modMsg } }));
        } else {
          const vips = (chatters.vips || []).map(v => v.login).filter(Boolean);
          const vipMsg = vips.length > 0
            ? `The VIPs of this channel are: ${vips.join(', ')}`
            : 'There are no VIPs of this channel.';
          window.dispatchEvent(new CustomEvent('twitch-system-message', { detail: { message: vipMsg } }));
        }
        return true;
      }
      // ────────────────────────────────────────────────────────
      // Suspicious User Management (Mod/Broadcaster)
      // ────────────────────────────────────────────────────────
      case 'monitor': {
        if (argsWithoutCommand.length >= 1) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('update_suspicious_user_status', { broadcasterId, targetUserId: user.id, status: 'ACTIVE_MONITORING' });
            addToast(`Now monitoring ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        addToast('Usage: /monitor <username>', 'info');
        return true;
      }
      case 'unmonitor': {
        if (argsWithoutCommand.length >= 1) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('update_suspicious_user_status', { broadcasterId, targetUserId: user.id, status: 'NO_TREATMENT' });
            addToast(`Stopped monitoring ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        addToast('Usage: /unmonitor <username>', 'info');
        return true;
      }
      case 'restrict': {
        if (argsWithoutCommand.length >= 1) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('update_suspicious_user_status', { broadcasterId, targetUserId: user.id, status: 'RESTRICTED' });
            addToast(`Restricted ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        addToast('Usage: /restrict <username>', 'info');
        return true;
      }
      case 'unrestrict': {
        if (argsWithoutCommand.length >= 1) {
          const username = argsWithoutCommand[0].replace('@', '');
          const user = await lookupUser(username);
          if (user) {
            await invoke('update_suspicious_user_status', { broadcasterId, targetUserId: user.id, status: 'NO_TREATMENT' });
            addToast(`Unrestricted ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        addToast('Usage: /unrestrict <username>', 'info');
        return true;
      }

      // ────────────────────────────────────────────────────────
      // Warn (Mod/Broadcaster) — uses Helix warn endpoint
      // ────────────────────────────────────────────────────────
      case 'warn': {
        if (argsWithoutCommand.length >= 2) {
          const username = argsWithoutCommand[0].replace('@', '');
          const reason = argsWithoutCommand.slice(1).join(' ');
          const user = await lookupUser(username);
          if (user) {
            await invoke('warn_chat_user', { broadcasterId, targetUserId: user.id, reason });
            addToast(`Warning sent to ${username}`, 'success');
          } else {
            addToast(`User ${username} not found`, 'error');
          }
          return true;
        }
        addToast('Usage: /warn <username> <reason>', 'info');
        return true;
      }

      // ────────────────────────────────────────────────────────
      // Shield Mode (Mod/Broadcaster)
      // ────────────────────────────────────────────────────────
      case 'shield': {
        await invoke('update_shield_mode', { broadcasterId, isActive: true });
        addToast('Shield mode activated', 'success');
        return true;
      }
      case 'shieldoff': {
        await invoke('update_shield_mode', { broadcasterId, isActive: false });
        addToast('Shield mode deactivated', 'success');
        return true;
      }

      // ────────────────────────────────────────────────────────
      // IRC passthrough — /me still works via IRC natively
      // ────────────────────────────────────────────────────────
      case 'me':
        return false; // Let IRC handle it

      // ────────────────────────────────────────────────────────
      // StreamNook-native QoL commands (client-side only)
      // ────────────────────────────────────────────────────────
      case 'clearmessages': {
        // Wipe the current pane's local message buffer. Purely visual — does
        // NOT call the moderator /clear command. ChatMessageList listens.
        window.dispatchEvent(new CustomEvent('streamnook-clear-local-chat'));
        emitSystemMessage('Cleared messages in this chat (visual only).');
        return true;
      }
      case 'openurl': {
        if (argsWithoutCommand.length === 0) {
          addToast('Usage: /openurl <url>', 'info');
          return true;
        }
        const target = argsWithoutCommand[0];
        try {
          // Bare-minimum URL guard so users don't accidentally shell-out odd
          // strings via this command. Allow http/https/twitch protocols.
          const parsed = new URL(target.includes('://') ? target : `https://${target}`);
          if (!['http:', 'https:'].includes(parsed.protocol)) {
            addToast(`Refused to open ${parsed.protocol} URL`, 'error');
            return true;
          }
          await openInBrowser(parsed.toString());
        } catch {
          addToast(`Not a valid URL: ${target}`, 'error');
        }
        return true;
      }
      case 'popout': {
        const channelLogin = (argsWithoutCommand[0] || broadcasterLogin || '').replace('@', '').toLowerCase();
        if (!channelLogin) {
          addToast('Usage: /popout [channel]', 'info');
          return true;
        }
        await openInBrowser(`https://www.twitch.tv/popout/${channelLogin}/chat?popout=`);
        return true;
      }
      case 'r': {
        const last = getLastWhisperFrom();
        const text = argsWithoutCommand.join(' ').trim();
        if (!last) {
          addToast('No whisper to reply to yet', 'info');
          return true;
        }
        if (!text) {
          addToast(`Usage: /r <message>  (replies to ${last.name})`, 'info');
          return true;
        }
        const user = await lookupUser(last.login);
        if (!user) {
          addToast(`Could not find ${last.name}`, 'error');
          return true;
        }
        await invoke('send_whisper', { toUserId: user.id, message: text });
        addToast(`Whisper sent to ${last.name}`, 'success');
        return true;
      }
      case 'clip': {
        try {
          await useAppStore.getState().createClip();
        } catch (err) {
          Logger.error('[Command Handler] /clip failed:', err);
          addToast('Could not create a clip', 'error');
        }
        return true;
      }
      case 'chatters': {
        if (!broadcasterId || !broadcasterLogin) {
          emitSystemMessage('No channel open.');
          return true;
        }
        try {
          const res = await invoke<unknown>('get_channel_chatters', {
            broadcasterId,
            channelLogin: broadcasterLogin,
          });
          const count = countChatters(res);
          emitSystemMessage(
            count === null
              ? 'Chatter count unavailable for this channel.'
              : `${count.toLocaleString()} chatter${count === 1 ? '' : 's'} connected.`,
          );
        } catch (err) {
          Logger.error('[Command Handler] /chatters failed:', err);
          emitSystemMessage('Chatter count needs moderator access on this channel.');
        }
        return true;
      }
      case 'ignore': {
        const target = (argsWithoutCommand[0] || '').replace('@', '').toLowerCase();
        if (!target) {
          addToast('Usage: /ignore <user>', 'info');
          return true;
        }
        await invoke('set_chat_user_hidden', { name: target, channelKey: null, hidden: true });
        emitSystemMessage(`Hiding ${target} everywhere. Undo in Settings > Chat > Hidden Messages.`);
        return true;
      }
      case 'settitle': {
        const title = argsWithoutCommand.join(' ').trim();
        if (!title || !broadcasterId) {
          addToast('Usage: /settitle <title>', 'info');
          return true;
        }
        try {
          await invoke('update_channel_info', { broadcasterId, title, gameName: null });
          addToast('Stream title updated', 'success');
        } catch (err) {
          addToast(typeof err === 'string' ? err : 'Could not update the title', 'error');
        }
        return true;
      }
      case 'setgame': {
        const gameName = argsWithoutCommand.join(' ').trim();
        if (!gameName || !broadcasterId) {
          addToast('Usage: /setgame <category>', 'info');
          return true;
        }
        try {
          await invoke('update_channel_info', { broadcasterId, title: null, gameName });
          addToast(`Category set to ${gameName}`, 'success');
        } catch (err) {
          addToast(typeof err === 'string' ? err : 'Could not update the category', 'error');
        }
        return true;
      }
      case 'overlay': {
        const channelLogin = (argsWithoutCommand[0] || broadcasterLogin || '').replace('@', '').toLowerCase();
        if (!channelLogin) {
          addToast('Usage: /overlay [channel]', 'info');
          return true;
        }
        try {
          const { openChatOverlayWindow } = await import('./chatOverlayWindow');
          let channelId: string | undefined;
          const currentStream = useAppStore.getState().currentStream;
          if (currentStream?.user_login?.toLowerCase() === channelLogin) {
            channelId = currentStream.user_id;
          } else {
            const lookup = await invoke<UserLookupResult | null>('get_user_by_login', { login: channelLogin }).catch(() => null);
            channelId = lookup?.id;
          }
          await openChatOverlayWindow({ channel: channelLogin, channelId });
        } catch (err) {
          Logger.error('[Command Handler] /overlay failed:', err);
          addToast('Failed to open chat overlay', 'error');
        }
        return true;
      }
      case 'popup': {
        const channelLogin = (argsWithoutCommand[0] || broadcasterLogin || '').replace('@', '').toLowerCase();
        if (!channelLogin) {
          addToast('Usage: /popup [channel]', 'info');
          return true;
        }
        try {
          // Dynamic import so this module doesn't pull in the WebviewWindow
          // dependency for users who never run this command.
          const { openMultiChatWindow } = await import('./multichatWindow');
          // Look up the user_id when the target isn't the current channel —
          // MultiChat panes prefer a stable id but degrade to login lookup.
          let channelId: string | undefined;
          const currentStream = useAppStore.getState().currentStream;
          if (currentStream?.user_login?.toLowerCase() === channelLogin) {
            channelId = currentStream.user_id;
          } else {
            const lookup = await invoke<UserLookupResult | null>('get_user_by_login', { login: channelLogin }).catch(() => null);
            channelId = lookup?.id;
          }
          // Chatterino parity: /popup always opens a NEW window.
          await openMultiChatWindow({ channel: channelLogin, channelId, newWindow: true });
        } catch (err) {
          Logger.error('[Command Handler] /popup failed:', err);
          addToast('Failed to open MultiChat window', 'error');
        }
        return true;
      }
      case 'uptime': {
        const currentStream = useAppStore.getState().currentStream;
        if (!currentStream?.started_at) {
          emitSystemMessage('This channel is offline.');
          return true;
        }
        const uptime = formatStreamUptime(currentStream.started_at);
        const channelName = currentStream.user_name || currentStream.user_login || 'This channel';
        emitSystemMessage(`${channelName} has been live for ${uptime}.`);
        return true;
      }
      case 'user':
      case 'usercard': {
        if (argsWithoutCommand.length === 0) {
          addToast(`Usage: /${command} <user>`, 'info');
          return true;
        }
        const username = argsWithoutCommand[0].replace('@', '');
        const user = await lookupUser(username);
        if (!user) {
          addToast(`User ${username} not found`, 'error');
          return true;
        }
        // Routed through a custom event so the active pane (main chat or any
        // MultiChat window) can decide where to open the card.
        window.dispatchEvent(new CustomEvent('streamnook-open-user-card', {
          detail: { userId: user.id, username: user.login, displayName: user.display_name },
        }));
        return true;
      }
      case 'refresh': {
        if (!broadcasterLogin || !broadcasterId) {
          addToast('/refresh: no active channel', 'error');
          return true;
        }
        try {
          const mod = await import('../stores/chatConnectionStore');
          const set = await mod.refreshChannelEmotes(broadcasterLogin, broadcasterId);
          const total = set
            ? set.twitch.length + set.bttv.length + set['7tv'].length + set.ffz.length
            : 0;
          emitSystemMessage(`Emotes refreshed (${total} loaded).`);
        } catch (err: unknown) {
          const errMsg = err instanceof Error ? err.message : String(err);
          addToast(`/refresh failed: ${errMsg}`, 'error');
        }
        return true;
      }
      case 'reload': {
        // Hard refresh of BOTH the stream and chat — the command-line twin of
        // the overlay Refresh button. /refresh (above) only busts the emote
        // cache; this restarts the video feed and reconnects/reloads chat.
        const app = useAppStore.getState();
        const stream = app.currentStream;
        if (!stream?.user_login || app.currentMediaType !== 'live') {
          addToast('/reload: no active live stream', 'error');
          return true;
        }
        addToast('Reloading stream and chat...', 'info');
        void app.reloadStreamAndChat();
        return true;
      }
      case 'remind': {
        const state = useAppStore.getState();
        const reminders = state.settings.reminders?.reminders ?? [];
        const result = parseRemindCommand(argsWithoutCommand, broadcasterLogin, broadcasterId);

        // Input-correction feedback stays a toast (matches /color, /w usage);
        // everything else prints into chat as a local system message so the
        // whole flow lives where you're typing.
        if (result.error) {
          addToast(result.error, 'info');
          return true;
        }
        if (result.help) {
          remindHelpLines().forEach(emitSystemMessage);
          return true;
        }
        if (result.list) {
          formatReminderList(reminders).forEach(emitSystemMessage);
          return true;
        }
        if (result.clear) {
          if (reminders.length === 0) {
            emitSystemMessage('You have no reminders to clear.');
            return true;
          }
          state.updateSettings({ ...state.settings, reminders: { reminders: [] } });
          emitSystemMessage(`Cleared ${reminders.length} reminder${reminders.length === 1 ? '' : 's'}.`);
          return true;
        }
        if (result.manage) {
          const { op, index } = result.manage;
          const i = index - 1;
          if (i < 0 || i >= reminders.length) {
            emitSystemMessage(`There's no reminder #${index}. Run /remind list to see them.`);
            return true;
          }
          if (op === 'remove') {
            state.updateSettings({
              ...state.settings,
              reminders: { reminders: reminders.filter((_, idx) => idx !== i) },
            });
            emitSystemMessage(`Removed reminder #${index}.`);
            return true;
          }
          const enabled = op === 'toggle' ? !reminders[i].enabled : op === 'enable';
          state.updateSettings({
            ...state.settings,
            reminders: { reminders: reminders.map((r, idx) => (idx === i ? { ...r, enabled } : r)) },
          });
          emitSystemMessage(`Reminder #${index} ${enabled ? 'enabled' : 'disabled'}.`);
          return true;
        }
        if (result.sendNow) {
          const sent = await fireReminderNow(result.sendNow.message, result.sendNow.repeat);
          if (!sent) addToast('/remind now: no active channel to post in', 'error');
          return true;
        }
        if (result.reminder) {
          state.updateSettings({
            ...state.settings,
            reminders: { reminders: [...reminders, result.reminder] },
          });
          emitSystemMessage(`${result.summary || 'Reminder set'} (now #${reminders.length + 1}). /remind list to manage.`);
          return true;
        }
        return true;
      }
      case 'nuke': {
        if (!broadcasterLogin) {
          addToast('/nuke: no active channel', 'error');
          return true;
        }
        if (!isUserModeratorOf(broadcasterLogin)) {
          addToast('/nuke is a moderator-only command', 'error');
          return true;
        }
        const raw = argsWithoutCommand.join(' ');
        // `/nuke stop` cancels any armed future windows on this channel. Checked
        // before parsing, since "stop" is not a valid pattern/action/window trio.
        if (raw.trim().toLowerCase() === 'stop') {
          const { stopActiveNukes } = await import('./nukeEngine');
          const stopped = stopActiveNukes(broadcasterLogin);
          addToast(
            stopped > 0
              ? `/nuke: stopped ${stopped} active window(s)`
              : '/nuke: nothing running on this channel',
            stopped > 0 ? 'success' : 'info',
          );
          return true;
        }
        const parsed = parseNukeArgs(raw);
        if ('error' in parsed) {
          addToast(parsed.error, 'error');
          return true;
        }
        try {
          const { matchedMessages, affectedUsers } = await executeNuke(
            broadcasterLogin,
            broadcasterId,
            parsed,
          );
          addToast(`/nuke: ${matchedMessages} message(s), ${affectedUsers} user(s)`, 'success');
        } catch (err: unknown) {
          const errMsg = err instanceof Error ? err.message : String(err);
          addToast(`/nuke failed: ${errMsg}`, 'error');
        }
        return true;
      }
      case 'undo': {
        if (!broadcasterLogin) {
          addToast('/undo: no active channel', 'error');
          return true;
        }
        if (!isUserModeratorOf(broadcasterLogin)) {
          addToast('/undo is a moderator-only command', 'error');
          return true;
        }
        try {
          await executeUndo(broadcasterLogin);
        } catch (err: unknown) {
          const errMsg = err instanceof Error ? err.message : String(err);
          addToast(`/undo failed: ${errMsg}`, 'error');
        }
        return true;
      }
      case 'banid': {
        if (argsWithoutCommand.length < 1) {
          addToast('Usage: /banid <userID> [reason]', 'info');
          return true;
        }
        const targetUserId = argsWithoutCommand[0];
        // Twitch user IDs are numeric. Reject obvious typos early so the user
        // gets a clearer error than the Helix 400.
        if (!/^\d+$/.test(targetUserId)) {
          addToast('User ID must be numeric — did you mean /ban?', 'error');
          return true;
        }
        const reason = argsWithoutCommand.slice(1).join(' ');
        try {
          await invoke('ban_user', { broadcasterId, targetUserId, duration: null, reason: reason || null });
          addToast(`Banned user ID ${targetUserId}`, 'success');
        } catch (err: unknown) {
          const errMsg = err instanceof Error ? err.message : String(err);
          addToast(`Ban failed: ${errMsg}`, 'error');
        }
        return true;
      }

      // ────────────────────────────────────────────────────────
      // Commands that show usage info or are client-side only
      // ────────────────────────────────────────────────────────
      // Poll / prediction builder. Opening the composer rather than parsing a
      // long argument list: the form validates against Twitch's limits and
      // previews the banner, which a one-liner can't.
      case 'poll':
      case 'prediction': {
        window.dispatchEvent(
          new CustomEvent('streamnook-open-poll-composer', {
            detail: { channel: broadcasterLogin, kind: command },
          }),
        );
        return true;
      }
      case 'endpoll':
      case 'cancelpoll': {
        if (!broadcasterId) {
          addToast(`/${command}: no active channel`, 'error');
          return true;
        }
        // Helix row shape: { id, title, status, choices: [...] }
        const active = await invoke<{ id?: string } | null>('get_active_poll', {
          broadcasterId,
        }).catch(() => null);
        if (!active?.id) {
          addToast('No poll is running right now', 'error');
          return true;
        }
        try {
          // TERMINATED keeps the result on screen; ARCHIVED hides it entirely.
          await invoke('end_poll', {
            broadcasterId,
            pollId: active.id,
            status: command === 'endpoll' ? 'TERMINATED' : 'ARCHIVED',
          });
          addToast(command === 'endpoll' ? 'Poll ended' : 'Poll cancelled', 'success');
        } catch (err: unknown) {
          addToast(typeof err === 'string' ? err : `/${command} failed`, 'error');
        }
        return true;
      }
      case 'lockprediction':
      case 'cancelprediction':
      case 'completeprediction': {
        if (!broadcasterId) {
          addToast(`/${command}: no active channel`, 'error');
          return true;
        }
        // Rust finds the open prediction, checks it, picks the winner (by
        // number or title) and ends it; this only shows the answer.
        try {
          const done = await invoke<string>('finish_prediction', {
            broadcasterId,
            action: command.replace('prediction', ''),
            outcome: argsWithoutCommand.join(' ').trim() || null,
          });
          addToast(done, 'success');
        } catch (err: unknown) {
          addToast(typeof err === 'string' ? err : `/${command} failed`, 'error');
        }
        return true;
      }
      case 'blockterm':
      case 'unblockterm': {
        if (!broadcasterId || !broadcasterLogin) {
          addToast(`/${command}: no active channel`, 'error');
          return true;
        }
        if (!isUserModeratorOf(broadcasterLogin)) {
          addToast(`/${command} is a moderator-only command`, 'error');
          return true;
        }
        const phrase = argsWithoutCommand.join(' ').trim();
        if (!phrase) {
          addToast(`/${command} needs a phrase`, 'error');
          return true;
        }
        try {
          if (command === 'blockterm') {
            await invoke('add_blocked_term', { broadcasterId, text: phrase });
            addToast(`Blocked "${phrase}"`, 'success');
          } else {
            await invoke('remove_blocked_term', { broadcasterId, text: phrase });
            addToast(`Unblocked "${phrase}"`, 'success');
          }
        } catch (err: unknown) {
          addToast(typeof err === 'string' ? err : `/${command} failed`, 'error');
        }
        return true;
      }
      case 'pin': {
        if (!broadcasterId || !broadcasterLogin) {
          addToast('/pin: no active channel', 'error');
          return true;
        }
        if (!isUserModeratorOf(broadcasterLogin)) {
          addToast('/pin is a moderator-only command', 'error');
          return true;
        }
        const { resolvePinTarget } = await import('./pinCommand');
        const resolved = resolvePinTarget(broadcasterLogin, argsWithoutCommand);
        if ('error' in resolved) {
          addToast(resolved.error, 'error');
          return true;
        }
        if (resolved.kind === 'send') {
          // Sent through the normal path (optimistic row, own-message
          // reconciliation); Rust pins it once Twitch has given it an id.
          const user = useAppStore.getState().currentUser;
          if (!user) {
            addToast('/pin: sign in to send a message', 'error');
            return true;
          }
          try {
            const { sendChannelMessage } = await import('../stores/chatConnectionStore');
            const outcome = await sendChannelMessage(
              broadcasterLogin,
              resolved.text,
              {
                username: user.login || user.username,
                displayName: user.display_name || user.username,
                userId: user.user_id,
                color: undefined,
                badges: '',
              },
              undefined,
              null,
              { pinAfterSend: true },
            );
            if (!outcome) {
              addToast('/pin: chat is not connected', 'error');
              return true;
            }
            // A dropped message is not pinned; a reason, when Twitch gave one,
            // is already shown in chat.
            if (!outcome.is_sent) {
              if (!outcome.drop_reason) addToast('Message not sent, so nothing was pinned', 'error');
              return true;
            }
            if (outcome.pin_error) {
              addToast(outcome.pin_error, 'error');
              return true;
            }
            usePinStore.getState().requestRefresh();
            addToast('Sent and pinned', 'success');
          } catch (err: unknown) {
            addToast(typeof err === 'string' ? err : '/pin failed', 'error');
          }
          return true;
        }
        try {
          await invoke('pin_chat_message', {
            broadcasterId,
            messageId: resolved.messageId,
            durationSeconds: resolved.durationSeconds,
          });
          usePinStore.getState().requestRefresh();
          addToast(
            resolved.durationSeconds
              ? `Pinned for ${resolved.durationSeconds}s`
              : 'Message pinned',
            'success',
          );
        } catch (err: unknown) {
          addToast(typeof err === 'string' ? err : '/pin failed', 'error');
        }
        return true;
      }
      case 'unpin': {
        if (!broadcasterId || !broadcasterLogin) {
          addToast('/unpin: no active channel', 'error');
          return true;
        }
        if (!isUserModeratorOf(broadcasterLogin)) {
          addToast('/unpin is a moderator-only command', 'error');
          return true;
        }
        try {
          // No message id: Rust unpins whatever is pinned in this channel.
          await invoke('unpin_chat_message', { broadcasterId });
          usePinStore.getState().requestRefresh();
          addToast('Message unpinned', 'success');
        } catch (err: unknown) {
          addToast(typeof err === 'string' ? err : '/unpin failed', 'error');
        }
        return true;
      }
      default:
        // The newer commands live in extendedCommands.ts. Anything neither
        // knows goes on to chat as text (Twitch still runs /me).
        return await runExtendedCommand({
          command,
          typed,
          args: argsWithoutCommand,
          rest: message.slice(1).trim().slice(typed.length).trim(),
          broadcasterId,
          broadcasterLogin,
          send: sendMessageToChat,
          depth,
          dispatch: (line: string) => dispatchScriptLine(line, broadcasterId, broadcasterLogin, depth + 1),
        });
    }
  } catch (err: unknown) {
    const errMsg = err instanceof Error ? err.message : String(err);
    Logger.error(`[Command Handler] Failed to execute /${command}:`, err);
    addToast(`Command failed: ${errMsg}`, 'error');
    return true; // We handled it (it failed, but we shouldn't send it to chat directly)
  }

  // A listed command that reached here was missing its arguments. Say how to
  // use it; never pass it to IRC, where Twitch runs no commands but /me.
  if (definition) {
    if (broadcasterLogin) noticeIn(broadcasterLogin, `Usage: ${definition.usage}`);
    addToast(`Usage: ${definition.usage}`, 'info');
    return true;
  }
  return false;
};


/**
 * One line of a script (/chain, /foreach, /timer, /runin) in its channel.
 * Everything it sends goes to that channel, paced by the send limiter and sent
 * from Rust, even when the chat box has moved on to another channel (or the
 * chat is closed) by the time a /timer fires. A command that goes to chat as
 * text (/me, or one nothing here knows) is sent the same way.
 */
export async function dispatchScriptLine(
  line: string,
  broadcasterId: string,
  broadcasterLogin: string,
  depth: number,
): Promise<boolean> {
  if (!line.startsWith('/')) return sendPaced(broadcasterLogin, line, broadcasterId);
  const handled = await handleSlashCommand(
    line,
    broadcasterId,
    broadcasterLogin,
    (msg) => void sendPaced(broadcasterLogin, msg, broadcasterId),
    depth,
  );
  return handled || sendPaced(broadcasterLogin, line, broadcasterId);
}

ensureLastWhisperListener();

/** Best-effort chatter count from either Helix chatters shape. */
function countChatters(res: unknown): number | null {
  if (!res || typeof res !== 'object') return null;
  const r = res as Record<string, unknown>;
  if (typeof r.total === 'number') return r.total;
  if (Array.isArray(r.data)) return r.data.length;
  if (Array.isArray(res)) {
    let n = 0;
    for (const group of res as unknown[]) {
      if (group && typeof group === 'object') {
        for (const v of Object.values(group as Record<string, unknown>)) {
          if (Array.isArray(v)) n += v.length;
        }
      }
    }
    return n;
  }
  return null;
}
