import { invoke } from '@tauri-apps/api/core';
import { ensureLastWhisperListener, getLastWhisperFrom } from './lastWhisper';
import { open as openExternalUrl } from '@tauri-apps/plugin-shell';
import { useAppStore } from '../stores/AppStore';
import { Logger } from './logger';
import {
  expandUserCommand,
  findUserCommand,
  formatStreamUptime,
  RESERVED_TRIGGERS,
  type TemplateContext,
} from './chatCommands';
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
  sendMessageToChat: (msg: string) => void
): Promise<boolean> => {
  if (!message.startsWith('/')) return false;

  const args = message.slice(1).trim().split(/\s+/);
  const command = args[0].toLowerCase();
  const argsWithoutCommand = args.slice(1);

  const addToast = useAppStore.getState().addToast;

  // User-defined slash commands. Built-in triggers (RESERVED_TRIGGERS) always
  // win — the switch below catches them regardless of what the user typed in
  // settings. Plain-text (non-slash) user commands are handled by ChatWidget
  // before this function is called.
  if (!RESERVED_TRIGGERS.has(command)) {
    const state = useAppStore.getState();
    const userCommand = findUserCommand(command, state.settings.chat_commands?.user_commands, true);
    if (userCommand) {
      const ctx = buildTemplateContext(argsWithoutCommand, broadcasterId, broadcasterLogin);
      const expansion = expandUserCommand(userCommand.expansion, ctx);
      if (expansion.missing_args.length > 0) {
        addToast(
          `/${command} expects argument${expansion.missing_args.length > 1 ? 's' : ''} ${expansion.missing_args
            .map((i) => `{${i}}`)
            .join(', ')}`,
          'error',
        );
        return true;
      }
      if (!expansion.text) {
        addToast(`/${command} expanded to an empty message`, 'error');
        return true;
      }
      sendMessageToChat(expansion.text);
      return true;
    }
  }

  try {
    switch (command) {
      // ────────────────────────────────────────────────────────
      // StreamNook-native: identify the music playing in the stream
      // ────────────────────────────────────────────────────────
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
          const username = argsWithoutCommand[0];
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
          const username = argsWithoutCommand[0];
          const duration = argsWithoutCommand.length > 1 ? parseInt(argsWithoutCommand[1], 10) : 600;
          const reason = argsWithoutCommand.slice(2).join(' ');
          const user = await lookupUser(username);
          if (user) {
            const safeDuration = isNaN(duration) ? 600 : duration;
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
          const username = argsWithoutCommand[0];
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
          await invoke('stop_chat');
          addToast('Disconnected from chat', 'info');
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
        const { withHiddenUser } = await import('./chatFilters');
        const st = useAppStore.getState();
        st.updateSettings({
          ...st.settings,
          chat_filters: withHiddenUser(st.settings.chat_filters, target, 'global', true),
        });
        emitSystemMessage(`Hiding ${target} everywhere. Undo in Settings > Chat > Hidden Users.`);
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
      case 'help': {
        const helpCmd = argsWithoutCommand[0] || '';
        window.dispatchEvent(new CustomEvent('twitch-system-message', {
          detail: { message: helpCmd
            ? `For help with /${helpCmd}, visit: https://help.twitch.tv/s/article/chat-commands`
            : 'Available commands: /mods, /vips, /color, /block, /unblock, /me, /w, /announce, and more. Use /help <command> for details.'
          }
        }));
        return true;
      }
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
        const active = await invoke<{
          prediction_id?: string;
          outcomes?: Array<{ id: string; title: string }>;
        } | null>('get_active_prediction', { channelLogin: broadcasterLogin }).catch(() => null);
        if (!active?.prediction_id) {
          addToast('No prediction is running right now', 'error');
          return true;
        }
        let status = 'LOCKED';
        let winningOutcomeId: string | null = null;
        if (command === 'cancelprediction') {
          status = 'CANCELED';
        } else if (command === 'completeprediction') {
          status = 'RESOLVED';
          // Accept either the outcome's position (1-based, how people say it
          // out loud) or its title.
          const arg = argsWithoutCommand.join(' ').trim();
          const outcomes = active.outcomes ?? [];
          if (!arg) {
            addToast('Say which outcome won, by number or title', 'error');
            return true;
          }
          const byIndex = Number(arg);
          const picked =
            Number.isInteger(byIndex) && byIndex >= 1 && byIndex <= outcomes.length
              ? outcomes[byIndex - 1]
              : outcomes.find((o) => o.title.toLowerCase() === arg.toLowerCase());
          if (!picked) {
            addToast(`No outcome matches "${arg}"`, 'error');
            return true;
          }
          winningOutcomeId = picked.id;
        }
        try {
          await invoke('end_prediction', {
            broadcasterId,
            predictionId: active.prediction_id,
            status,
            winningOutcomeId,
          });
          addToast(
            status === 'LOCKED'
              ? 'Prediction locked'
              : status === 'CANCELED'
                ? 'Prediction cancelled, points refunded'
                : 'Prediction paid out',
            'success',
          );
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
        try {
          await invoke('pin_chat_message', {
            broadcasterId,
            messageId: resolved.messageId,
            durationSeconds: resolved.durationSeconds,
          });
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
          await invoke('unpin_chat_message', { broadcasterId });
          addToast('Message unpinned', 'success');
        } catch (err: unknown) {
          addToast(typeof err === 'string' ? err : '/unpin failed', 'error');
        }
        return true;
      }
      case 'vote':
      case 'gift':
      case 'deletepoll':
      case 'goal':
      case 'raidbrowser':
      case 'requests':
      case 'sharedchat':
        // These are handled natively by the Twitch web client and cannot be replicated.
        // Let them fall through to IRC as a best-effort attempt.
        return false;
      default:
        return false;
    }
  } catch (err: unknown) {
    const errMsg = err instanceof Error ? err.message : String(err);
    Logger.error(`[Command Handler] Failed to execute /${command}:`, err);
    addToast(`Command failed: ${errMsg}`, 'error');
    return true; // We handled it (it failed, but we shouldn't send it to chat directly)
  }
  
  return false;
};


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
