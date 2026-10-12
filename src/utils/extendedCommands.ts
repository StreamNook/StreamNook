// The slash commands added on top of the original switch in commandHandler.ts.
// Presentation only: every Twitch, GQL, file and limit decision is a Rust
// command; this file reads what was typed, calls the right one, and prints
// the answer in the chat it was typed in.

import { invoke } from '@tauri-apps/api/core';
import { open as openExternalUrl } from '@tauri-apps/plugin-shell';
import { useAppStore } from '../stores/AppStore';
import {
  getChannelEmotes,
  hardRefreshChannel,
  injectSystemMessage,
  sendChannelMessage,
  systemSourceFor,
  useChatConnectionStore,
} from '../stores/chatConnectionStore';
import { useLocalChatViewStore, type LocalView } from '../stores/localChatViewStore';
import { COMMAND_DEFINITIONS, findCommandDefinition, type CommandDefinition } from './chatCommands';
import { isUserModeratorOf } from './nukeEngine';
import { Logger } from './logger';
import { IS_MOBILE } from './platform';

export interface CommandContext {
  /** The command's own name, after aliases (`b` arrives as `ban`). */
  command: string;
  /** What was typed (`b`, `ffz`, `automod_approve`), for aliases that carry meaning. */
  typed: string;
  args: string[];
  /** Everything after the command word, spacing kept. */
  rest: string;
  broadcasterId: string;
  broadcasterLogin: string;
  /** Send plain text to this chat the normal way. */
  send: (msg: string) => void;
  /** Run a line (a command or plain text) in this chat; used by /timer, /chain... */
  dispatch: (line: string) => Promise<boolean>;
  /** How deep in /chain, /foreach, /timer this run is. */
  depth?: number;
}

// ── shared helpers ───────────────────────────────────────────────────────────

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

const errText = (err: unknown) => (err instanceof Error ? err.message : String(err));

/** A login from what was typed: `@Name`, `#name` or `name`, lowercased. */
export function cleanLogin(raw: string | undefined): string {
  return (raw ?? '').trim().replace(/^[@#]+/, '').toLowerCase();
}

/** A Kick/YouTube/TikTok chat: its key is `provider:channel`, a Twitch one a bare login. */
function isProviderChat(channel: string): boolean {
  return channel.includes(':');
}

function me() {
  return useAppStore.getState().currentUser;
}

function isBroadcasterOf(login: string): boolean {
  const user = me();
  const own = (user?.login || user?.username || '').toLowerCase();
  return !!own && own === login.toLowerCase();
}

function isStaffOf(login: string): boolean {
  return isBroadcasterOf(login) || isUserModeratorOf(login);
}

interface UserLookup {
  id: string;
  login: string;
  display_name: string;
  profile_image_url?: string;
}

async function lookupUser(login: string): Promise<UserLookup | null> {
  try {
    return await invoke<UserLookup>('get_user_by_login', { login });
  } catch {
    return null;
  }
}

/** Ask for the same command twice within 20 seconds before something that
 *  is hard to undo. True means "not confirmed yet, a prompt was shown".
 *  Only a command typed in the chat box counts (`depth` 0): from /chain,
 *  /foreach, /timer or /runin the command is refused, so a script can never
 *  confirm itself. */
const pendingConfirms = new Map<string, number>();
const CONFIRM_WINDOW_MS = 20_000;
export function needsConfirmation(key: string, channel: string, prompt: string, depth = 0, name = ''): boolean {
  if (depth > 0) {
    const what = name ? `/${name}` : 'This command';
    noticeIn(channel, `${what} asks you to confirm, so it can't run from /chain, /foreach, /timer or /runin. Type it in the chat box.`);
    return true;
  }
  const now = Date.now();
  const at = pendingConfirms.get(key);
  if (at && now - at < CONFIRM_WINDOW_MS) {
    pendingConfirms.delete(key);
    return false;
  }
  pendingConfirms.set(key, now);
  noticeIn(channel, `${prompt} Run the same command again within 20 seconds to confirm.`);
  return true;
}

function userInfo() {
  const u = me();
  if (!u) return null;
  return {
    username: u.login || u.username,
    displayName: u.display_name || u.username,
    userId: u.user_id,
    badges: '',
  };
}

/** A line about a send to `channel`, where the person will see it: in that
 *  chat when it is open here, as a toast when it is not (a /timer can fire
 *  after its chat was closed or switched away from). */
function sendNotice(channel: string, message: string): void {
  if (channel && useChatConnectionStore.getState().channels.get(channel)?.isConnected) noticeIn(channel, message);
  else toast(message, 'error');
}

/** Send a scripted line (/foreach, /timer, /chain, /runin) to `channel`.
 *  Rust waits for a slot from the chat send limiter, so scripts stay inside
 *  Twitch's limits, then sends as the signed-in account over Helix, which
 *  needs no open chat: a timer still posts after its chat is gone.
 *  `channelId` saves Rust a lookup when it is known. */
export async function sendPaced(channel: string, text: string, channelId?: string): Promise<boolean> {
  const key = channel.toLowerCase();
  if (!me()) {
    toast('Sign in to Twitch to send messages', 'error');
    return false;
  }
  try {
    const outcome = await invoke<{ is_sent: boolean; drop_reason: string | null } | null>('chat_send_paced', {
      channel: key,
      channelId: channelId || null,
      text,
    });
    if (outcome && !outcome.is_sent) {
      sendNotice(key, `Not sent to ${key}: ${outcome.drop_reason ?? 'Twitch dropped it'}`);
    }
    return !!outcome?.is_sent;
  } catch (refusal) {
    sendNotice(key, `Not sent to ${key || 'chat'}: ${errText(refusal)}`);
    return false;
  }
}

/** "30s", "5m", "1h30m", "2d", "1w", or a bare number of seconds, as milliseconds. */
export function parseDelay(raw: string | undefined): number | null {
  const s = (raw ?? '').trim().toLowerCase();
  if (!s) return null;
  if (/^\d+$/.test(s)) return parseInt(s, 10) * 1000;
  const m = s.match(/^(?:(\d+)w)?(?:(\d+)d)?(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?$/);
  if (!m || !(m[1] || m[2] || m[3] || m[4] || m[5])) return null;
  const [w, d, h, min, sec] = m.slice(1).map((v) => (v ? +v : 0));
  return ((((w * 7 + d) * 24 + h) * 60 + min) * 60 + sec) * 1000;
}

function usage(def: CommandDefinition | undefined, fallback: string): void {
  toast(`Usage: ${def?.usage ?? fallback}`, 'info');
}

function twitchUrlOpen(url: string): Promise<void> {
  return openExternalUrl(url);
}

const MAX_SCRIPT_DEPTH = 3;
const MAX_FOREACH = 20;
const MAX_CHAIN = 10;
const MAX_TIMERS = 20;
const MAX_TIMER_MS = 24 * 3600 * 1000;
const timers = new Map<string, Set<ReturnType<typeof setTimeout>>>();

// ── /help ────────────────────────────────────────────────────────────────────

/** The lines `/help [command]` prints, from the same definitions the command
 *  menu shows. */
export function helpLines(arg: string | undefined, mobile = IS_MOBILE): string[] {
  const name = (arg ?? '').trim().replace(/^\//, '').toLowerCase();
  if (!name) {
    const byCategory = new Map<string, string[]>();
    for (const c of COMMAND_DEFINITIONS) {
      // The phone lists only what works on the phone.
      if (c.hint || (mobile && c.desktopOnly)) continue;
      const list = byCategory.get(c.category) ?? [];
      list.push(`/${c.name}`);
      byCategory.set(c.category, list);
    }
    const lines = ['Commands (try /help <command> for one of them, or the command button left of the emote picker):'];
    byCategory.forEach((names, cat) => lines.push(`${cat}: ${names.join(' ')}`));
    return lines;
  }
  const def = findCommandDefinition(name);
  if (!def) return [`There is no /${name} command. /help lists them all.`];
  if (mobile && def.desktopOnly) return [`/${def.name} works in the desktop app.`];
  const lines = [`${def.usage}: ${def.description}`];
  if (def.details) lines.push(def.details);
  if (def.aliases?.length) lines.push(`Also works as ${def.aliases.map((a) => `/${a}`).join(', ')}`);
  if (def.examples?.length) lines.push(`For example: ${def.examples.join('  ')}`);
  return lines;
}

// ── the commands ─────────────────────────────────────────────────────────────

type Handler = (ctx: CommandContext, def: CommandDefinition | undefined) => Promise<void>;

const say = (ctx: CommandContext, line: string) => noticeIn(ctx.broadcasterLogin, line);
const sayAll = (ctx: CommandContext, lines: string[]) => lines.forEach((l) => say(ctx, l));

function needChannel(ctx: CommandContext): boolean {
  if (ctx.broadcasterId && ctx.broadcasterLogin) return true;
  toast(`/${ctx.typed}: open a channel first`, 'error');
  return false;
}

function needStaff(ctx: CommandContext): boolean {
  if (isStaffOf(ctx.broadcasterLogin)) return true;
  toast(`/${ctx.typed} is for moderators and the broadcaster`, 'error');
  return false;
}

function needBroadcaster(ctx: CommandContext): boolean {
  if (isBroadcasterOf(ctx.broadcasterLogin)) return true;
  toast(`/${ctx.typed} is for the broadcaster of this channel`, 'error');
  return false;
}

async function lines(ctx: CommandContext, cmd: string, args: Record<string, unknown>): Promise<void> {
  const out = await invoke<string | string[]>(cmd, args);
  sayAll(ctx, Array.isArray(out) ? out : [out]);
}

const LOCAL_VIEWS: Record<string, { view: LocalView; on: boolean; label: string }> = {
  localmod: { view: 'mod', on: true, label: 'Showing only messages from moderators and the broadcaster' },
  localmodoff: { view: 'mod', on: false, label: 'Moderator-only view off' },
  localsub: { view: 'sub', on: true, label: 'Showing only messages from subscribers, moderators and the broadcaster' },
  localsuboff: { view: 'sub', on: false, label: 'Subscriber-only view off' },
  localascii: { view: 'ascii', on: true, label: 'Showing only messages written in plain ASCII' },
  localasciioff: { view: 'ascii', on: false, label: 'ASCII-only view off' },
};

const EMOTE_PROVIDERS: Record<string, { key: 'twitch' | 'bttv' | '7tv' | 'ffz'; label: string }> = {
  twitchemotes: { key: 'twitch', label: 'Twitch' },
  bttv: { key: 'bttv', label: 'BTTV' },
  bttvemotes: { key: 'bttv', label: 'BTTV' },
  seventv: { key: '7tv', label: '7TV' },
  ffz: { key: 'ffz', label: 'FFZ' },
  ffzglobal: { key: 'ffz', label: 'FFZ' },
};

const SQUISHY =
  'notsquishY WHEN YOU NEED HIM notsquishY IN A JIFFY notsquishY USE THIS EMOTE notsquishY TO SUMMON SQUISHY notsquishY';

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

function getPath(obj: unknown, path: string[]): unknown {
  let cur: unknown = obj;
  for (const p of path) {
    if (cur === null || typeof cur !== 'object') return undefined;
    cur = (cur as Record<string, unknown>)[p];
  }
  return cur;
}

function withPath<T>(obj: T, path: string[], value: unknown): T {
  const root = structuredClone(obj) as Record<string, unknown>;
  let cur = root;
  for (const p of path.slice(0, -1)) {
    const next = cur[p];
    cur[p] = next && typeof next === 'object' ? next : {};
    cur = cur[p] as Record<string, unknown>;
  }
  cur[path[path.length - 1]] = value;
  return root as T;
}

/** Parse a `/setting set` value: JSON when it reads as JSON, text otherwise. */
export function parseSettingValue(raw: string): unknown {
  const t = raw.trim();
  try {
    return JSON.parse(t);
  } catch {
    return t;
  }
}

function kindOf(v: unknown): string {
  if (v === null || v === undefined) return 'empty';
  if (Array.isArray(v)) return 'list';
  return typeof v;
}

const handlers: Record<string, Handler> = {
  async help(ctx) {
    sayAll(ctx, helpLines(ctx.args[0]));
  },

  async vote(ctx, def) {
    if (!needChannel(ctx)) return;
    if (!ctx.rest.trim()) return usage(def, '/vote <number or option>');
    try {
      say(ctx, await invoke<string>('slash_vote', { channelId: ctx.broadcasterId, choice: ctx.rest.trim() }));
    } catch (err) {
      toast(errText(err), 'error');
    }
  },

  async gift(ctx) {
    if (!needChannel(ctx)) return;
    if (isBroadcasterOf(ctx.broadcasterLogin)) {
      toast("You can't gift subs in your own channel", 'info');
      return;
    }
    const qty = parseInt(ctx.args[0] ?? '', 10);
    await invoke('open_subscribe_window', {
      channelLogin: ctx.broadcasterLogin,
      title: `Gift subs to ${ctx.broadcasterLogin}`,
      provider: 'twitch',
    });
    say(
      ctx,
      Number.isFinite(qty) && qty > 0
        ? `Opened ${ctx.broadcasterLogin}'s sub page. Choose "Gift a sub" and pick ${qty}. Payment happens on Twitch.`
        : `Opened ${ctx.broadcasterLogin}'s sub page. Choose "Gift a sub" there. Payment happens on Twitch.`,
    );
  },

  async rules(ctx) {
    const channel = cleanLogin(ctx.args[0]) || ctx.broadcasterLogin;
    if (!channel) return toast('/rules: open a channel first', 'error');
    await lines(ctx, 'slash_chat_rules', { channel });
  },

  async followage(ctx) {
    const [a, b] = ctx.args.map(cleanLogin);
    const channel = b || ctx.broadcasterLogin;
    if (!channel) return toast('/followage: open a channel first', 'error');
    await lines(ctx, 'slash_followage', { channel, user: a || null });
  },

  async follows(ctx) {
    const channel = cleanLogin(ctx.args[0]) || ctx.broadcasterLogin;
    if (!channel) return toast('/follows: open a channel first', 'error');
    await lines(ctx, 'slash_follower_count', { channel });
  },

  async accountage(ctx) {
    const user = cleanLogin(ctx.args[0]) || (me()?.login ?? '').toLowerCase();
    if (!user) return toast('Usage: /accountage <user>', 'info');
    await lines(ctx, 'slash_account_age', { user });
  },

  async viewers(ctx) {
    const channel = cleanLogin(ctx.args[0]) || ctx.broadcasterLogin;
    if (!channel) return toast('/viewers: open a channel first', 'error');
    await lines(ctx, 'slash_viewer_count', { channel });
  },

  async founders(ctx) {
    const channel = cleanLogin(ctx.args[0]) || ctx.broadcasterLogin;
    if (!channel) return toast('/founders: open a channel first', 'error');
    await lines(ctx, 'slash_founders', { channel });
  },

  async namehistory(ctx, def) {
    const user = cleanLogin(ctx.args[0]);
    if (!user) return usage(def, '/namehistory <user>');
    await lines(ctx, 'slash_name_history', { user });
  },

  async logs(ctx, def) {
    const login = cleanLogin(ctx.args[0]);
    if (!login) return usage(def, '/logs <user>');
    if (!ctx.broadcasterLogin) return toast('/logs: open a channel first', 'error');
    await lines(ctx, 'slash_recent_messages', { channel: ctx.broadcasterLogin, login, limit: 10 });
  },

  async reply(ctx, def) {
    const login = cleanLogin(ctx.args[0]);
    const text = ctx.args.slice(1).join(' ').trim();
    if (!login || !text) return usage(def, '/reply <@user> <message>');
    if (!needChannel(ctx)) return;
    const hit = await invoke<{ id: string; display_name: string } | null>('slash_last_message', {
      channel: ctx.broadcasterLogin,
      login,
    });
    if (!hit) return toast(`No recent message from ${login} in this chat to reply to`, 'error');
    const info = userInfo();
    if (!info) return toast('Sign in to Twitch to reply', 'error');
    const outcome = await sendChannelMessage(ctx.broadcasterLogin, text, info, hit.id);
    if (!outcome) toast('Chat is not connected', 'error');
  },

  async redeem(ctx) {
    if (!needChannel(ctx)) return;
    window.dispatchEvent(new CustomEvent('streamnook-open-channel-points', { detail: { channel: ctx.broadcasterLogin } }));
  },

  async copy(ctx, def) {
    if (!ctx.rest.trim()) return usage(def, '/copy <text>');
    await invoke('slash_copy_text', { text: ctx.rest.trim() });
    toast('Copied', 'success');
  },

  async unignore(ctx, def) {
    const target = cleanLogin(ctx.args[0]);
    if (!target) return usage(def, '/unignore <user>');
    await invoke('set_chat_user_hidden', { name: target, channelKey: null, hidden: false });
    say(ctx, `${target} is no longer hidden.`);
  },

  async join(ctx, def) {
    const login = cleanLogin(ctx.args[0]);
    if (!login) return usage(def, '/join <channel>');
    const user = await lookupUser(login);
    if (!user) return toast(`There is no Twitch channel named ${login}`, 'error');
    const { dockChat, setDockPanelOpen } = await import('../stores/chatDockStore');
    await dockChat(
      {
        provider: 'twitch',
        login: user.login,
        channel_id: user.id,
        display_name: user.display_name || user.login,
        avatar_url: user.profile_image_url ?? null,
      },
      true,
    );
    setDockPanelOpen(true);
    toast(`Joined ${user.display_name || user.login}'s chat in the dock`, 'success');
  },

  async part(ctx) {
    const login = cleanLogin(ctx.args[0]) || ctx.broadcasterLogin;
    const { undockChat, useChatDockStore } = await import('../stores/chatDockStore');
    const docked = useChatDockStore.getState().chats.find((c) => c.provider === 'twitch' && c.login.toLowerCase() === login);
    if (!docked) return toast(`${login || 'That chat'} is not in your chat dock`, 'info');
    await undockChat(`twitch:${docked.login.toLowerCase()}`);
    toast(`Left ${docked.display_name || docked.login}'s chat`, 'success');
  },

  async reconnect(ctx) {
    if (!ctx.broadcasterLogin) return toast('/reconnect: open a channel first', 'error');
    // The refresh empties this chat, so the notice comes after it; a toast
    // covers the wait.
    toast('Reconnecting chat...', 'info');
    await hardRefreshChannel(ctx.broadcasterLogin, ctx.broadcasterId || null);
    say(ctx, 'Chat reconnected.');
  },

  async echo(ctx, def) {
    if (!ctx.rest.trim()) return usage(def, '/echo <text>');
    say(ctx, ctx.rest.trim());
  },

  async say(ctx, def) {
    if (!ctx.rest.trim()) return usage(def, '/say <text>');
    ctx.send(ctx.rest.trim());
  },

  async search() {
    const { getChatSearchController } = await import('../keybindings/chatSearchController');
    const controller = getChatSearchController();
    if (!controller) return toast('Chat search is not available here', 'info');
    controller.openSearch();
  },

  async setcolor(ctx, def) {
    const login = cleanLogin(ctx.args[0]);
    const value = (ctx.args[1] ?? '').trim();
    if (!login || !value) return usage(def, '/setcolor <user> <color|off>');
    const user = await lookupUser(login);
    if (!user) return toast(`There is no Twitch user named ${login}`, 'error');
    const off = /^(off|none|reset|clear)$/i.test(value);
    if (!off && !/^#?[0-9a-f]{6}$/i.test(value) && !CSS.supports?.('color', value)) {
      return toast(`"${value}" is not a colour. Use a name like orange or a hex like #ff8800`, 'error');
    }
    const color = off ? null : /^[0-9a-f]{6}$/i.test(value) ? `#${value}` : value;
    const { setUserColor } = await import('./userChatOverrides');
    setUserColor(user.id, user.login, color);
    say(ctx, off ? `${user.display_name}'s name colour is back to their own.` : `${user.display_name}'s name now shows in ${color} for you.`);
  },

  async setname(ctx, def) {
    const login = cleanLogin(ctx.args[0]);
    const nickname = ctx.args.slice(1).join(' ').trim();
    if (!login || !nickname) return usage(def, '/setname <user> <nickname|off>');
    const user = await lookupUser(login);
    if (!user) return toast(`There is no Twitch user named ${login}`, 'error');
    const off = /^(off|none|reset|clear)$/i.test(nickname);
    const { setUserNickname } = await import('./userChatOverrides');
    setUserNickname(user.id, user.login, off ? null : nickname);
    say(ctx, off ? `${user.display_name} shows under their own name again.` : `${user.display_name} now shows as "${nickname}" for you.`);
  },

  async addstreamhighlight(ctx) {
    if (!ctx.broadcasterLogin) return toast('/addstreamhighlight: open a channel first', 'error');
    say(ctx, await invoke<string>('slash_add_stream_highlight', { channel: ctx.broadcasterLogin, comment: ctx.rest.trim() || null }));
  },

  async openstreamhighlights() {
    await invoke('slash_open_stream_highlights');
  },

  async favorite(ctx) {
    await toggleFavorite(ctx, true);
  },

  async unfavorite(ctx) {
    await toggleFavorite(ctx, false);
  },

  async tts(ctx, def) {
    const text = ctx.rest.trim();
    if (!text) return usage(def, '/tts <text>');
    const synth = typeof window !== 'undefined' ? window.speechSynthesis : undefined;
    if (!synth) return toast('Text to speech is not available on this system', 'error');
    synth.speak(new SpeechSynthesisUtterance(text));
  },

  async screenshot(ctx) {
    const video = findPlayerVideo();
    if (!video) return toast('No stream is playing to take a screenshot of', 'error');
    const rect = await screenRectOf(video);
    const path = await invoke<string>('save_stream_screenshot', { ...rect, channel: ctx.broadcasterLogin || null });
    say(ctx, `Screenshot saved: ${path}`);
  },

  async '7tv'(ctx, def) {
    await sevenTvCommand(ctx, def);
  },

  async emoteinfo(ctx) {
    if (!ctx.broadcasterLogin) return toast('Open a channel first', 'error');
    const set = getChannelEmotes(ctx.broadcasterLogin);
    if (!set) return say(ctx, 'Emotes for this channel have not loaded yet.');
    const only = EMOTE_PROVIDERS[ctx.typed] ?? EMOTE_PROVIDERS[cleanLogin(ctx.args[0])];
    const rows = only ? [only] : Object.values(EMOTE_PROVIDERS).filter((p, i, all) => all.findIndex((q) => q.key === p.key) === i);
    const parts = rows.map((p) => `${p.label} ${(set[p.key] ?? []).length.toLocaleString()}`);
    say(ctx, `Emotes loaded for ${ctx.broadcasterLogin}: ${parts.join(', ')}. /refresh fetches them again.`);
  },

  async streamlink(ctx) {
    const channel = ctx.broadcasterLogin;
    if (!channel) return toast('/streamlink: open a channel first', 'error');
    say(ctx, await invoke<string>('slash_open_streamlink', { channel, quality: ctx.args[0] ?? null }));
  },

  async raw(ctx, def) {
    if (!ctx.rest.trim()) return usage(def, '/raw <IRC line>');
    await invoke('slash_send_raw_irc', { line: ctx.rest.trim() });
    say(ctx, `Sent: ${ctx.rest.trim()}`);
  },

  async fakemsg(ctx, def) {
    if (!ctx.rest.trim()) return usage(def, '/fakemsg <IRC PRIVMSG line>');
    await invoke('slash_inject_fake_message', { line: ctx.rest.trim() });
  },

  async 'debug-env'(ctx) {
    const [version, system, settingsDir, logDir] = await Promise.all([
      invoke<string>('get_app_version').catch(() => '?'),
      invoke<string>('get_system_info').catch(() => '?'),
      invoke<string>('get_settings_dir').catch(() => '?'),
      invoke<string>('get_chat_log_dir').catch(() => '?'),
    ]);
    sayAll(ctx, [
      `StreamNook ${version} (${import.meta.env.DEV ? 'development' : 'release'} build) on ${system}`,
      `Settings: ${settingsDir}`,
      `Chat logs: ${logDir}`,
    ]);
  },

  async 'debug-chat'(ctx) {
    const log = await invoke<string[]>('get_chat_lifecycle_log');
    const tail = log.slice(-8);
    sayAll(ctx, tail.length ? ['Recent chat connection events:', ...tail] : ['No chat connection events yet.']);
  },

  async 'debug-eventsub'(ctx) {
    const [connected, session] = await Promise.all([
      invoke<boolean>('is_eventsub_connected').catch(() => false),
      invoke<string | null>('get_eventsub_session_id').catch(() => null),
    ]);
    say(ctx, connected ? `EventSub is connected (session ${session ?? 'unknown'}).` : 'EventSub is not connected.');
  },

  async 'debug-logs'() {
    await invoke('open_logs_folder');
  },

  async 'debug-logging'(ctx, def) {
    const v = (ctx.args[0] ?? '').toLowerCase();
    if (v !== 'on' && v !== 'off') return usage(def, '/debug-logging <on|off>');
    const state = useAppStore.getState();
    await state.updateSettings({ ...state.settings, error_reporting_enabled: v === 'on' });
    say(ctx, v === 'on' ? 'Detailed logging is on.' : 'Detailed logging is off.');
  },

  async 'debug-limits'(ctx) {
    if (!ctx.broadcasterLogin) return toast('Open a channel first', 'error');
    const b = await invoke<{
      role: string;
      per_window: number;
      window_seconds: number;
      used_in_window: number;
      spacing_ms: number;
      slow_seconds: number;
    }>('chat_send_budget', { channel: ctx.broadcasterLogin });
    say(
      ctx,
      `You are a ${b.role} here: ${b.per_window} messages per ${b.window_seconds}s (${b.used_in_window} used), at least ${b.spacing_ms} ms apart${b.slow_seconds ? `, slow mode ${b.slow_seconds}s` : ''}.`,
    );
  },

  async setting(ctx, def) {
    const [verb, path, ...valueParts] = ctx.args;
    const keys = (path ?? '').split('.').filter(Boolean);
    if (!verb || !keys.length || !['get', 'set', 'reset'].includes(verb.toLowerCase())) {
      return usage(def, '/setting <get|set|reset> <name> [value]');
    }
    const state = useAppStore.getState();
    const current = getPath(state.settings, keys);
    const defaults = await invoke<unknown>('get_default_settings').catch(() => null);
    const fallback = getPath(defaults, keys);
    if (current === undefined && fallback === undefined) {
      return toast(`There is no setting called ${keys.join('.')}`, 'error');
    }
    const name = keys.join('.');
    if (verb.toLowerCase() === 'get') {
      const shown = JSON.stringify(current ?? null);
      return say(ctx, `${name} = ${shown.length > 300 ? `${shown.slice(0, 300)}...` : shown}`);
    }
    let next: unknown;
    if (verb.toLowerCase() === 'reset') {
      if (fallback === undefined) return toast(`${name} has no default to go back to`, 'error');
      next = fallback;
    } else {
      if (!valueParts.length) return usage(def, '/setting set <name> <value>');
      next = parseSettingValue(valueParts.join(' '));
      const want = kindOf(current ?? fallback);
      if (want !== 'empty' && kindOf(next) !== want) {
        return toast(`${name} takes a ${want}, not a ${kindOf(next)}`, 'error');
      }
    }
    await state.updateSettings(withPath(state.settings, keys, next));
    say(ctx, `${name} = ${JSON.stringify(next)}`);
  },

  async proc(ctx, def) {
    if (!ctx.rest.trim()) return usage(def, '/proc <program> [arguments]');
    say(ctx, await invoke<string>('slash_run_program', { commandLine: ctx.rest.trim() }));
  },

  async exporttext(ctx, def) {
    const file = ctx.args[0];
    const text = ctx.args.slice(1).join(' ');
    if (!file || !text.trim()) return usage(def, '/exporttext <file> <text>');
    const path = await invoke<string>('slash_export_text', { file, text });
    say(ctx, `Added to ${path}`);
  },

  async openfile(ctx) {
    await invoke('slash_open_export', { file: ctx.args[0] ?? null });
  },

  async timer(ctx, def) {
    const key = ctx.broadcasterLogin || '_';
    if ((ctx.args[0] ?? '').toLowerCase() === 'stop') {
      const set = timers.get(key);
      const n = set?.size ?? 0;
      set?.forEach((t) => clearTimeout(t));
      timers.delete(key);
      return say(ctx, n ? `Cancelled ${n} timer${n === 1 ? '' : 's'}.` : 'No timers are waiting here.');
    }
    const ms = parseDelay(ctx.args[0]);
    const line = ctx.args.slice(1).join(' ').trim();
    if (ms === null || !line) return usage(def, '/timer <delay> <command or message>');
    if (ms > MAX_TIMER_MS) return toast('A timer can wait up to 24 hours', 'error');
    if ((ctx.depth ?? 0) >= MAX_SCRIPT_DEPTH) return toast('Timers can only nest three deep', 'error');
    const set = timers.get(key) ?? new Set();
    if (set.size >= MAX_TIMERS) return toast(`At most ${MAX_TIMERS} timers can wait at once`, 'error');
    const handle = setTimeout(() => {
      set.delete(handle);
      void ctx.dispatch(line);
    }, ms);
    set.add(handle);
    timers.set(key, set);
    say(ctx, `Runs in ${ctx.args[0]}: ${line}  (/timer stop cancels)`);
  },

  async runin(ctx, def) {
    const login = cleanLogin(ctx.args[0]);
    const line = ctx.args.slice(1).join(' ').trim();
    if (!login || !line) return usage(def, '/runin <channel> <command or message>');
    if ((ctx.depth ?? 0) >= MAX_SCRIPT_DEPTH) return toast('Commands can only nest three deep', 'error');
    const user = await lookupUser(login);
    if (!user) return toast(`There is no Twitch channel named ${login}`, 'error');
    // Sends go out over Helix from Rust, so the chat does not need to be open.
    const { dispatchScriptLine } = await import('./commandHandler');
    await dispatchScriptLine(line, user.id, user.login.toLowerCase(), (ctx.depth ?? 0) + 1);
  },

  async foreach(ctx, def) {
    const items = (ctx.args[0] ?? '').split(',').map((s) => s.trim()).filter(Boolean);
    const template = ctx.args.slice(1).join(' ').trim();
    if (!items.length || !template) return usage(def, '/foreach <a,b,c> <command with $>');
    if (!template.includes('$')) return toast('Put $ where each item goes, for example /foreach a,b /shoutout $', 'error');
    if (items.length > MAX_FOREACH) return toast(`/foreach takes up to ${MAX_FOREACH} items`, 'error');
    if ((ctx.depth ?? 0) >= MAX_SCRIPT_DEPTH) return toast('Commands can only nest three deep', 'error');
    for (const item of items) {
      await ctx.dispatch(template.split('$').join(item));
    }
  },

  async chain(ctx, def) {
    const steps = ctx.rest.split('|').map((s) => s.trim()).filter(Boolean);
    if (steps.length < 2) return usage(def, '/chain <command> | <command>');
    if (steps.length > MAX_CHAIN) return toast(`/chain takes up to ${MAX_CHAIN} steps`, 'error');
    if ((ctx.depth ?? 0) >= MAX_SCRIPT_DEPTH) return toast('Commands can only nest three deep', 'error');
    for (const step of steps) {
      await ctx.dispatch(step);
    }
  },

  async shrug(ctx) {
    const text = ctx.rest.trim();
    ctx.send(text ? `${text} ¯\\_(ツ)_/¯` : '¯\\_(ツ)_/¯');
  },

  async lurk(ctx) {
    const text = ctx.rest.trim();
    // /me is Twitch's; Kick and YouTube would post it as the letters "/me".
    const said = text ? `is now lurking: ${text}` : 'is now lurking';
    ctx.send(isProviderChat(ctx.broadcasterLogin) ? said : `/me ${said}`);
  },

  async squishy(ctx) {
    ctx.send(SQUISHY);
  },

  async barrelroll() {
    if (!playWindowEffect('barrelroll')) toast('Skipped: reduced motion is on', 'info');
  },

  async party() {
    if (!playWindowEffect('party')) toast('Skipped: reduced motion is on', 'info');
  },

  async localmod(ctx) {
    if (!ctx.broadcasterLogin) return toast('Open a channel first', 'error');
    const v = LOCAL_VIEWS[ctx.typed] ?? LOCAL_VIEWS[ctx.command];
    useLocalChatViewStore.getState().setView(ctx.broadcasterLogin, v.view, v.on);
    say(ctx, `${v.label}. Only you see this.`);
  },

  // ── moderation ──
  async purge(ctx, def) {
    const login = cleanLogin(ctx.args[0]);
    if (!login) return usage(def, '/purge <user> [reason]');
    if (!needChannel(ctx) || !needStaff(ctx)) return;
    const user = await lookupUser(login);
    if (!user) return toast(`There is no Twitch user named ${login}`, 'error');
    const reason = ctx.args.slice(1).join(' ').trim();
    await invoke('ban_user', { broadcasterId: ctx.broadcasterId, targetUserId: user.id, duration: 1, reason: reason || null });
    toast(`Purged ${user.display_name}'s messages`, 'success');
  },

  async delete(ctx, def) {
    if (!ctx.args[0]) return usage(def, '/delete <message id | @user>');
    if (!needChannel(ctx) || !needStaff(ctx)) return;
    toast(
      await invoke<string>('slash_delete_message', {
        broadcasterId: ctx.broadcasterId,
        channel: ctx.broadcasterLogin,
        target: ctx.args[0],
      }),
      'success',
    );
  },

  async automod(ctx, def) {
    let verb = (ctx.args[0] ?? '').toLowerCase();
    let id = ctx.args[1];
    if (ctx.typed === 'automod_approve' || ctx.typed === 'automod_deny') {
      id = ctx.args[0];
      verb = ctx.typed === 'automod_approve' ? 'allow' : 'deny';
    }
    const allow = ['allow', 'approve', 'yes'].includes(verb);
    const deny = ['deny', 'reject', 'no'].includes(verb);
    if (!allow && !deny) return usage(def, '/automod <allow|deny> [message id]');
    if (!needChannel(ctx) || !needStaff(ctx)) return;
    say(ctx, await invoke<string>('slash_automod', { channel: ctx.broadcasterLogin, messageId: id ?? null, allow }));
  },

  async lowtrust(ctx) {
    if (!needChannel(ctx) || !needStaff(ctx)) return;
    await lines(ctx, 'slash_low_trust', { channel: ctx.broadcasterLogin });
  },

  async modlogs(ctx) {
    if (!needChannel(ctx) || !needStaff(ctx)) return;
    const state = useAppStore.getState();
    const on = !state.settings.show_mod_logs;
    await state.updateSettings({ ...state.settings, show_mod_logs: on });
    toast(on ? 'Mod log shown' : 'Mod log hidden', 'success');
  },

  async requests(ctx) {
    if (!needChannel(ctx) || !needStaff(ctx)) return;
    await twitchUrlOpen(`https://www.twitch.tv/popout/${ctx.broadcasterLogin}/reward-queue`);
  },

  // ── broadcaster ──
  async goal(ctx) {
    if (!needChannel(ctx) || !needBroadcaster(ctx)) return;
    await twitchUrlOpen(`https://dashboard.twitch.tv/u/${ctx.broadcasterLogin}/community/goals`);
    say(ctx, 'Opened your Creator Goals page. Twitch only lets goals be made and edited there.');
  },

  async sharedchat(ctx) {
    if (!needChannel(ctx)) return;
    await lines(ctx, 'slash_shared_chat', { broadcasterId: ctx.broadcasterId, channel: ctx.broadcasterLogin });
    if (isBroadcasterOf(ctx.broadcasterLogin)) {
      await twitchUrlOpen(`https://dashboard.twitch.tv/u/${ctx.broadcasterLogin}/stream-manager`);
      say(ctx, 'Opened your Stream Manager: Shared Chat starts from Stream Together there.');
    }
  },

  async raidbrowser(ctx) {
    await lines(ctx, 'slash_raid_candidates', {});
  },

  async raidsend(ctx) {
    if (!needChannel(ctx) || !needBroadcaster(ctx)) return;
    await invoke('slash_raid_now', { broadcasterId: ctx.broadcasterId });
    toast('Raid sent', 'success');
  },

  async massunban(ctx) {
    if (!needChannel(ctx) || !needBroadcaster(ctx)) return;
    if ((ctx.args[0] ?? '').toLowerCase() === 'stop') {
      const stopped = await invoke<boolean>('slash_mass_unban_stop', { broadcasterId: ctx.broadcasterId });
      return say(ctx, stopped ? 'Stopping the mass unban...' : 'No mass unban is running.');
    }
    if (needConfirm(ctx, `massunban:${ctx.broadcasterId}`, 'This lifts EVERY permanent ban in your channel and cannot be undone.')) return;
    await listenMassUnban(ctx.broadcasterLogin, ctx.broadcasterId);
    await invoke('slash_mass_unban_start', { broadcasterId: ctx.broadcasterId });
    say(ctx, 'Mass unban started, about three a second. /massunban stop ends it.');
  },

  async leadmod(ctx, def) {
    const login = cleanLogin(ctx.args[0]);
    if (!login) return usage(def, `/${ctx.command} <user>`);
    if (!needChannel(ctx) || !needBroadcaster(ctx)) return;
    say(ctx, await invoke<string>('slash_lead_mod', { broadcasterId: ctx.broadcasterId, target: login, assign: ctx.command === 'leadmod' }));
  },

  async editor(ctx, def) {
    const login = cleanLogin(ctx.args[0]);
    if (!login) return usage(def, `/${ctx.command} <user>`);
    if (!needChannel(ctx) || !needBroadcaster(ctx)) return;
    const add = ctx.command === 'editor';
    if (add && needConfirm(ctx, `editor:${login}`, `Editors can change your stream title, category, ads and more. Make ${login} an editor?`)) return;
    say(ctx, await invoke<string>('slash_editor', { broadcasterId: ctx.broadcasterId, target: login, add }));
  },
};

handlers.unleadmod = handlers.leadmod;
handlers.uneditor = handlers.editor;
for (const name of Object.keys(LOCAL_VIEWS)) handlers[name] = handlers.localmod;

function needConfirm(ctx: CommandContext, key: string, prompt: string): boolean {
  return needsConfirmation(key, ctx.broadcasterLogin, prompt, ctx.depth ?? 0, ctx.typed);
}

let massUnbanUnlisten: (() => void) | null = null;
async function listenMassUnban(channel: string, broadcasterId: string): Promise<void> {
  if (massUnbanUnlisten) return;
  const { listen } = await import('@tauri-apps/api/event');
  massUnbanUnlisten = await listen<{ broadcaster_id: string; state: string; message: string }>(
    'mass-unban-progress',
    (e) => {
      if (e.payload.broadcaster_id !== broadcasterId) return;
      noticeIn(channel, e.payload.message);
      if (e.payload.state !== 'running') {
        massUnbanUnlisten?.();
        massUnbanUnlisten = null;
      }
    },
  );
}

async function toggleFavorite(ctx: CommandContext, add: boolean): Promise<void> {
  const login = cleanLogin(ctx.args[0]) || ctx.broadcasterLogin;
  if (!login) return toast(`/${ctx.typed}: name a channel or open one`, 'error');
  const user = await lookupUser(login);
  if (!user) return toast(`There is no Twitch channel named ${login}`, 'error');
  const state = useAppStore.getState();
  const isFav = state.isFavoriteStreamer(user.id);
  if (isFav === add) {
    return toast(add ? `${user.display_name} is already a favorite` : `${user.display_name} is not a favorite`, 'info');
  }
  await state.toggleFavoriteStreamer(user.id, {
    id: user.id,
    provider: 'twitch',
    channel: user.login,
    display_name: user.display_name,
    avatar: user.profile_image_url,
    added_at: new Date().toISOString(),
  });
  toast(add ? `${user.display_name} added to favorites` : `${user.display_name} removed from favorites`, 'success');
}

async function sevenTvCommand(ctx: CommandContext, def: CommandDefinition | undefined): Promise<void> {
  const verb = (ctx.args[0] ?? '').toLowerCase();
  const editor = await import('../services/seventvEditorService');
  if (verb === 'search') {
    const q = ctx.args.slice(1).join(' ').trim();
    if (!q) return usage(def, '/7tv search <name>');
    const page = await editor.searchDirectory(q, { perPage: 8 });
    if (!page.emotes.length) return say(ctx, `No 7TV emotes match "${q}".`);
    return say(ctx, `7TV emotes for "${q}": ${page.emotes.map((e) => `${e.defaultName} (${e.id})`).join(', ')}. Add one with /7tv add <id>.`);
  }
  if (!['add', 'remove', 'rename'].includes(verb)) return usage(def, '/7tv <search|add|remove|rename> ...');
  if (!ctx.broadcasterLogin) return toast('Open a channel first', 'error');
  const channels = await editor.getEditableChannels();
  const target = channels.find((c) => c.username.toLowerCase() === ctx.broadcasterLogin || c.twitchId === ctx.broadcasterId);
  if (!target || !target.perms.manageEmotes) {
    return toast(`You can't edit ${ctx.broadcasterLogin}'s 7TV emotes. Sign in to 7TV with an editor account in Settings`, 'error');
  }
  const setId = target.activeEmoteSetId;
  if (!setId) return toast(`${ctx.broadcasterLogin} has no active 7TV emote set`, 'error');
  const findInSet = async (name: string) => {
    const page = await editor.getSetEmotes(setId, 1, 50, name);
    return page.emotes.find((e) => e.alias === name) ?? page.emotes.find((e) => e.alias.toLowerCase() === name.toLowerCase());
  };
  if (verb === 'add') {
    const input = ctx.args[1];
    const alias = ctx.args[2];
    if (!input) return usage(def, '/7tv add <emote id, link or name> [alias]');
    let emote = await editor.resolveEmote(input);
    if (!emote) {
      const page = await editor.searchDirectory(input, { perPage: 20 });
      emote = page.emotes.find((e) => e.defaultName === input) ?? page.emotes.find((e) => e.defaultName.toLowerCase() === input.toLowerCase()) ?? null;
    }
    if (!emote) return toast(`No 7TV emote called "${input}". Try /7tv search ${input}`, 'error');
    await editor.addEmote(setId, emote.id, alias ? { alias } : undefined);
    return say(ctx, `Added ${alias ?? emote.defaultName} to ${target.displayName}'s 7TV emotes.`);
  }
  const name = ctx.args[1];
  if (!name) return usage(def, `/7tv ${verb} <emote name>${verb === 'rename' ? ' <new name>' : ''}`);
  const inSet = await findInSet(name);
  if (!inSet) return toast(`${name} is not in ${target.displayName}'s 7TV emotes`, 'error');
  if (verb === 'remove') {
    await editor.removeEmote(setId, inSet.emoteId, inSet.alias);
    return say(ctx, `Removed ${inSet.alias} from ${target.displayName}'s 7TV emotes.`);
  }
  const next = ctx.args[2];
  if (!next) return usage(def, '/7tv rename <emote name> <new name>');
  await editor.renameEmote(setId, inSet.emoteId, next, inSet.alias);
  say(ctx, `Renamed ${inSet.alias} to ${next}.`);
}

/** Commands this module owns, by canonical name. */
export const EXTENDED_COMMANDS: ReadonlySet<string> = new Set(Object.keys(handlers));

/** Commands that work the same in any platform's chat (no Twitch API). */
export const PLATFORM_NEUTRAL: ReadonlySet<string> = new Set([
  'help', 'copy', 'echo', 'say', 'tts', 'shrug', 'lurk', 'squishy', 'barrelroll', 'party',
  'exporttext', 'openfile', 'proc', 'screenshot', 'setting', 'timer', 'chain', 'foreach',
  'debug-env', 'debug-chat', 'debug-eventsub', 'debug-logs', 'debug-logging',
  'openstreamhighlights',
]);

/** Run `ctx.command` when this module owns it. False means "not mine". */
export async function runExtendedCommand(ctx: CommandContext): Promise<boolean> {
  const handler = handlers[ctx.command];
  if (!handler) return false;
  const def = findCommandDefinition(ctx.command);
  try {
    await handler(ctx, def);
  } catch (err) {
    Logger.error(`[Commands] /${ctx.typed} failed:`, err);
    toast(errText(err) || `/${ctx.typed} failed`, 'error');
  }
  return true;
}
