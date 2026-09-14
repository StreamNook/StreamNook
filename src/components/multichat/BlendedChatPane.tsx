// BlendedChatPane — one continuous, time-ordered feed merging EVERY open source
// in a MultiChat window (e.g. a streamer's Twitch chat + their Kick chat, or two
// Twitch channels). Each message keeps its own rendering (baked emote segments +
// native badges) and is prefixed with its source provider's logo (ChatMessageList
// `showSource`). Emotes are resolved at parse time in Rust, so a mixed-source
// array needs no per-message emote context — we pass `emotes={null}`.
//
// The composer sends to any subset of the blended sources, chosen from a themed
// checkbox picker grouped by provider: tick a whole provider or individual
// channels, then one send fans out to every ticked channel.

import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import ChatMessageList from '../ChatMessageList';
import { ProviderLogo } from '../ProviderLogo';
import { useChatConnectionStore, sendChannelMessage, injectSystemMessage, systemSourceFor } from '../../stores/chatConnectionStore';
import { useChatUserStore } from '../../stores/chatUserStore';
import { useAppStore } from '../../stores/AppStore';
import { makeKey, parseKey } from '../../utils/providerKey';
import { openProfilePopup } from '../../utils/openProfilePopup';
import { PROVIDERS, PROVIDER_IDS, type ProviderId } from '../../types/providers';
import { parseMessage, type BackendChatMessage } from '../../services/twitchChat';
import { initializeBadgeCache } from '../../services/twitchBadges';
import type { ModerationContext } from '../../hooks/useTwitchChat';
import { usePlatformAccountStore } from '../../stores/platformAccountStore';
import HypeTrainBanner from '../HypeTrainBanner';
import { useBlendedHypeTrains } from './useBlendedHypeTrains';
import { Logger } from '../../utils/logger';
import { Tooltip } from '../ui/Tooltip';

interface BlendedChannel {
  channel: string;
  provider?: ProviderId;
  channelName: string;
}

const noop = () => {};

// Minimum spacing between pause/resume transitions (mirrors ChatWidget's
// PAUSE_SETTLE_MS): real gestures are hundreds of ms apart, so this is invisible,
// but it caps the machine-speed scroll/auto-scroll oscillation a fast chat produces.
// `force` transitions (Resume button, reply-jump) bypass it.
const PAUSE_SETTLE_MS = 120;

const provOf = (c: BlendedChannel): ProviderId => c.provider ?? 'twitch';
// Stable per-source key used by the picker, the merge match, and the send router.
const sourceKey = (c: BlendedChannel) => `${provOf(c)}::${c.channel.toLowerCase()}`;

// Common sortable epoch-ms from either a structured message (`timestamp`, which
// is ISO-UTC for Kick and an epoch for Twitch) or a raw IRC string (`tmi-sent-ts`).
function tsOf(m: string | BackendChatMessage): number {
  const t = typeof m === 'string' ? m.match(/tmi-sent-ts=(\d+)/)?.[1] ?? '' : m.timestamp ?? '';
  if (!t) return 0;
  if (/^\d+$/.test(t)) return Number(t);
  const d = Date.parse(t);
  return Number.isNaN(d) ? 0 : d;
}

// Send `text` to one source. With `reply`, route a reply through that provider's
// own mechanism: Twitch and Kick thread natively off the parent message id; YouTube
// has no threaded reply, so we @mention the recipient instead.
async function sendTo(
  c: BlendedChannel,
  text: string,
  reply?: { parentId: string; parentUser: string },
): Promise<void> {
  const prov = provOf(c);
  if (prov === 'twitch') {
    // Route Twitch sends through the shared store path the rest of the app uses. It
    // sends via Helix with the real broadcaster + sender ids AND adds the optimistic
    // copy to the slice (so the message also shows in the feed). The previous raw
    // invoke passed null ids, which made the backend skip Helix and fall back to an
    // IRC write that doesn't deliver here, so blended sends silently went nowhere.
    const cu = useAppStore.getState().currentUser;
    if (!cu?.user_id) return;
    await sendChannelMessage(
      c.channel,
      text,
      {
        username: cu.login || cu.username,
        displayName: cu.display_name || cu.username,
        userId: cu.user_id,
      },
      reply?.parentId,
    );
  } else {
    // YouTube live chat has no reply threads, so a reply becomes an @mention.
    const isYouTubeReply = prov === 'youtube' && !!reply;
    const outcome = await invoke<{
      message_id: string | null;
      is_sent: boolean;
      drop_reason: string | null;
    }>('provider_send_message', {
      provider: prov,
      channel: c.channel.toLowerCase(),
      text: isYouTubeReply ? `@${reply!.parentUser} ${text}` : text,
      replyTo: isYouTubeReply ? null : reply?.parentId ?? null,
    });
    // The platform can accept the request and still refuse the message. Say so
    // in the pane and rethrow so the composer can restore what was typed.
    if (outcome && outcome.is_sent === false) {
      const reason = outcome.drop_reason || 'Message not sent';
      const key = `${prov}:${c.channel.toLowerCase()}`;
      injectSystemMessage(key, `Your message was not sent: ${reason}`, undefined, systemSourceFor(key));
      throw new Error(reason);
    }
  }
}

// Small themed checkbox (checked / indeterminate / empty).
function Check({ checked, indeterminate }: { checked: boolean; indeterminate?: boolean }) {
  const on = checked || indeterminate;
  return (
    <span
      className="flex h-4 w-4 shrink-0 items-center justify-center rounded-[4px] border transition-colors"
      style={{
        borderColor: on ? 'var(--color-accent)' : 'rgba(255,255,255,0.25)',
        backgroundColor: on ? 'var(--color-accent)' : 'transparent',
      }}
    >
      {checked ? (
        <svg className="h-3 w-3 text-black" viewBox="0 0 20 20" fill="currentColor">
          <path
            fillRule="evenodd"
            d="M16.7 5.3a1 1 0 010 1.42l-7.5 7.5a1 1 0 01-1.42 0L3.3 9.74a1 1 0 011.42-1.42l3.07 3.07 6.79-6.79a1 1 0 011.42 0z"
            clipRule="evenodd"
          />
        </svg>
      ) : indeterminate ? (
        <span className="h-[2px] w-2 rounded bg-black" />
      ) : null}
    </span>
  );
}

export function BlendedChatPane({
  channels,
  mode = 'all',
  readOnly = false,
  transparent = false,
}: {
  channels: BlendedChannel[];
  /** 'mentions' keeps only rows the Rust rule engine stamped as a mention,
   *  a reply to us, or a highlight match. */
  mode?: 'all' | 'mentions';
  /** No composer: the overlay window is a viewer, not a place to type. */
  readOnly?: boolean;
  /** No own background: the host paints the (glass) ground. */
  transparent?: boolean;
}) {
  // Re-render when any of THIS blend's sources change. Summing the per-channel
  // counters (instead of the global revision) keeps the O(all sources) reconcile
  // below from re-running on flushes of channels this pane doesn't show.
  const revision = useChatConnectionStore((s) =>
    channels.reduce((sum, c) => {
      // Slice keys are the acquisition keys: bare login for Twitch, composite
      // provider:channel otherwise (mirrors the store's channel map).
      const prov = provOf(c);
      const sliceKey =
        prov === 'twitch' ? c.channel.toLowerCase() : makeKey(prov, c.channel);
      return sum + (s.revisionByChannel[sliceKey] ?? 0);
    }, 0),
  );
  // Twitch Hype Trains across the blended Twitch sources (blended mounts no per-pane
  // poller, so this drives both the banner here and the activity-feed rows).
  const hypeTrains = useBlendedHypeTrains(channels);
  // A level-up's confetti rains over the whole feed, so the shared banner portals
  // it into this element.
  const [feedEl, setFeedEl] = useState<HTMLElement | null>(null);

  // Incremental, append-only merge. The feed is ordered by FIRST-seen arrival and
  // only ever grows at the bottom (YouTube is polled, so a send-time sort would slot
  // its late messages mid-feed and thrash the scroll; freezing each slot on first
  // sight keeps everything on screen still). Blended mode used to re-derive AND
  // re-sort the entire combined feed on every frame any source got a message, which
  // pins the single webview main thread. Most visibly, a clip played over the feed
  // stutters. Instead we keep a persistent ordered list of ids and reconcile it
  // against the live slices each tick: append genuinely-new ids (send-time-sorted as
  // one batch), refresh in-place upgrades (own-message repaint / IRC echo / Helix-id
  // stamp keep the SAME id but swap the slot reference), and drop ids that have left
  // every slice (per-channel cap eviction, or a source removed from the blend). No
  // per-frame re-sort, and the rendered array keeps a STABLE identity on any tick
  // that changed nothing, so the memoized rows + list bail instead of re-rendering.
  const orderRef = useRef<string[]>([]);
  const idToMsgRef = useRef<Map<string, string | BackendChatMessage>>(new Map());
  // messageId -> the source it came from, so a right-click reply routes to the right
  // channel/account (a raw Twitch IRC line doesn't carry its own slug). Persistent +
  // read through a ref by the row callbacks so their identity stays stable.
  const idToChannelRef = useRef<Map<string, BlendedChannel>>(new Map());
  // Monotonic counter for the "N new since paused" badge. Order now lives in
  // orderRef, so this no longer drives placement, only the unread count.
  const seqRef = useRef<{ next: number }>({ next: 0 });
  // Cached render outputs. Their references only change when their contents do, so a
  // quiet tick hands the memoized list the exact same props and it skips the work.
  const renderCacheRef = useRef<{
    messages: (string | BackendChatMessage)[];
    deleted: Set<string>;
    cleared: Map<string, { context: ModerationContext; affectedMessageIds: Set<string> }>;
  }>({ messages: [], deleted: new Set(), cleared: new Map() });

  const { messages, deletedMessageIds, clearedUserContexts } = useMemo(() => {
    const store = useChatConnectionStore.getState();
    // Match by the STORE MAP KEY via parseKey (the source of truth): a Kick slice
    // is keyed `kick:slug` and its own `.channel` field holds that composite key,
    // not the bare slug. parseKey normalizes both bare Twitch (`xqc`) and composite.
    const open = new Set(channels.map(sourceKey));
    const byKey = new Map(channels.map((c) => [sourceKey(c), c] as const));
    const keyOf = (m: string | BackendChatMessage) =>
      typeof m === 'string' ? m.match(/id=([^;]+)/)?.[1] ?? m : m.id;
    const order = orderRef.current;
    const idToMsg = idToMsgRef.current;
    const idToChannel = idToChannelRef.current;

    let structuralChange = false; // appended/removed -> messages identity must change
    let refChange = false; // an on-screen row's reference upgraded in place

    // One pass over the open slices: record presence, queue newcomers, refresh
    // in-place upgrades. Newcomers are gathered in store-iteration order so that
    // equal-timestamp ties resolve the same way the old full-rebuild did. This
    // reconcile is idempotent (driven off the slices, keyed by id), so React 18
    // StrictMode's double-invoke in dev is a harmless no-op the second time.
    const present = new Set<string>();
    const newcomers: Array<{ id: string; m: string | BackendChatMessage; ch?: BlendedChannel }> = [];
    for (const [key, slice] of store.channels.entries()) {
      const pk = parseKey(key);
      const skey = `${pk.provider}::${pk.channel.toLowerCase()}`;
      if (!open.has(skey)) continue;
      const ch = byKey.get(skey);
      for (const m of slice.messages) {
        const id = keyOf(m);
        if (!id) continue; // unkeyable; can't track/dedupe/remove it reliably
        if (present.has(id)) continue; // a dup id across slices renders once (the list guards too)
        present.add(id);
        const prev = idToMsg.get(id);
        if (prev === undefined) {
          newcomers.push({ id, m, ch });
        } else if (prev !== m) {
          // Same id, new slot reference: an in-place upgrade (own repaint / echo /
          // Helix-id stamp). Refresh the held reference so the row repaints.
          idToMsg.set(id, m);
          refChange = true;
          if (ch) {
            idToChannel.set(id, ch);
            const tagId = typeof m !== 'string' ? m.tags?.['id'] : undefined;
            if (tagId && tagId !== id) idToChannel.set(tagId, ch);
          }
        }
      }
    }

    // Removals: ids we hold that no longer appear in any open slice (cap eviction,
    // or the source was removed from the blend). `present` already includes this
    // tick's newcomers (still absent from idToMsg), so a size comparison can't tell
    // us anything, so scan the held ids directly. Compact the order array in one pass
    // only when something was actually dropped.
    let removedAny = false;
    for (const id of Array.from(idToMsg.keys())) {
      if (!present.has(id)) {
        idToMsg.delete(id);
        idToChannel.delete(id);
        removedAny = true;
      }
    }
    if (removedAny) {
      let w = 0;
      for (let r = 0; r < order.length; r++) {
        if (idToMsg.has(order[r])) order[w++] = order[r];
      }
      order.length = w;
      structuralChange = true;
    }

    // Append newcomers as one send-time-ordered batch (Array.sort is stable, so
    // equal timestamps keep their store-iteration order).
    if (newcomers.length) {
      newcomers.sort((a, b) => tsOf(a.m) - tsOf(b.m));
      for (const x of newcomers) {
        idToMsg.set(x.id, x.m);
        order.push(x.id);
        seqRef.current.next++;
        if (x.ch) {
          idToChannel.set(x.id, x.ch);
          const tagId = typeof x.m !== 'string' ? x.m.tags?.['id'] : undefined;
          if (tagId && tagId !== x.id) idToChannel.set(tagId, x.ch);
        }
      }
      structuralChange = true;
    }

    // Moderation sets are FLAGGED on the slice (never spliced from messages), so
    // they stay cheap to re-collect; reuse the prior reference when unchanged so a
    // quiet tick doesn't re-render the list.
    const cache = renderCacheRef.current;
    const deleted = new Set<string>();
    const cleared = new Map<string, { context: ModerationContext; affectedMessageIds: Set<string> }>();
    for (const [key, slice] of store.channels.entries()) {
      const pk = parseKey(key);
      const skey = `${pk.provider}::${pk.channel.toLowerCase()}`;
      if (!open.has(skey)) continue;
      slice.deletedMessageIds?.forEach((id: string) => deleted.add(id));
      slice.clearedUserContexts?.forEach(
        (v: { context: ModerationContext; affectedMessageIds: Set<string> }, k: string) => cleared.set(k, v),
      );
    }
    const deletedSame =
      deleted.size === cache.deleted.size && [...deleted].every((id) => cache.deleted.has(id));
    const clearedSame =
      cleared.size === cache.cleared.size && [...cleared.keys()].every((k) => cache.cleared.has(k));
    if (!deletedSame) cache.deleted = deleted;
    if (!clearedSame) cache.cleared = cleared;

    // Rebuild the rendered array only when the set or a reference actually changed;
    // otherwise hand back the identical reference so the memoized rows + list bail.
    if (structuralChange || refChange || cache.messages.length !== order.length) {
      cache.messages = order.map((id) => idToMsg.get(id) as string | BackendChatMessage);
    }
    return {
      messages: cache.messages,
      deletedMessageIds: cache.deleted,
      clearedUserContexts: cache.cleared,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [channels, revision]);

  // Stable view of the current feed + source map for the row callbacks, so those
  // callbacks keep a fixed identity (they read the ref) and don't defeat the row
  // memo by changing every tick.
  const messagesRef = useRef(messages);
  messagesRef.current = messages;

  const shownMessages = useMemo(() => {
    if (mode !== 'mentions') return messages;
    return messages.filter(
      (m) =>
        typeof m !== 'string' &&
        (m.metadata?.is_mentioned === true ||
          m.metadata?.is_reply_to_me === true ||
          !!m.metadata?.highlight),
    );
  }, [messages, mode]);

  const getMessageId = useCallback(
    (m: string | BackendChatMessage) => (typeof m === 'string' ? m.match(/id=([^;]+)/)?.[1] ?? null : m.id),
    [],
  );

  // The blended pane renders ChatMessage directly, bypassing ChatWidget — which is what
  // normally loads chatter cosmetics (7TV paint/badge + third-party badges) into
  // chatUserStore AND populates the Twitch badge metadata (global mod/staff/turbo +
  // per-channel subscriber/bits) that parseBadges reads. In its own popout window those
  // module caches start empty, so without doing it here the merged feed shows only
  // baked-URL badges (Kick/YouTube) and no Twitch native or 7TV badges/paints.
  //
  // Global Twitch badges load once up front (cheap, disk-cached). Channel badges +
  // chatter cosmetics resolve as each NEW message/chatter is first seen; addUser dedupes
  // and fetches once per user, with the same provider namespacing ChatWidget uses.
  useEffect(() => {
    void initializeBadgeCache();
  }, []);
  const cosmeticsSeenRef = useRef<Set<string>>(new Set());
  const badgeChannelsRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    const addUser = useChatUserStore.getState().addUser;
    const seen = cosmeticsSeenRef.current;
    for (const m of messages) {
      const mid = getMessageId(m);
      if (!mid || seen.has(mid)) continue;
      seen.add(mid);
      let userId: string | undefined;
      let username: string | undefined;
      let displayName: string | undefined;
      let color: string | undefined;
      let provider: ProviderId = 'twitch';
      let channelId: string | undefined;
      let channelName = '';
      if (typeof m === 'string') {
        const parsed = parseMessage(m);
        userId = parsed.tags.get('user-id');
        username = parsed.username;
        displayName = parsed.tags.get('display-name') || parsed.username;
        color = parsed.color;
        channelId = parsed.tags.get('source-room-id') || parsed.tags.get('room-id');
      } else {
        userId = m.tags?.['user-id'] || m.user_id;
        username = m.username;
        displayName = m.display_name || m.username;
        color = m.color;
        provider = (m.provider as ProviderId) || 'twitch';
        channelId = m.tags?.['source-room-id'] || m.tags?.['room-id'];
        channelName = m.channel ? parseKey(m.channel).channel : '';
      }
      // Load this Twitch channel's subscriber/bits badge set once (global set is
      // already warming from the mount effect above).
      if (provider === 'twitch' && channelId && !badgeChannelsRef.current.has(channelId)) {
        badgeChannelsRef.current.add(channelId);
        void initializeBadgeCache(channelId);
      }
      if (!userId || !username) continue;
      addUser(
        {
          userId: provider === 'twitch' ? userId : `${provider}:${userId}`,
          username,
          displayName: displayName || username,
          color: color || '#9147FF',
        },
        channelId ? { channelId, channelName } : undefined,
      );
    }
    // Bound the seen set to messages still present (slices are capped).
    if (seen.size > messages.length * 2 + 128) {
      const present = new Set(
        messages.map((m) => getMessageId(m)).filter((id): id is string => !!id),
      );
      for (const id of seen) if (!present.has(id)) seen.delete(id);
    }
  }, [messages, getMessageId]);

  const paneRef = useRef<HTMLDivElement>(null);

  // Pause, mirroring ChatWidget's stable implementation: one rate-limited mutator +
  // grace periods stop the rapid pause/resume flapping a fast chat would otherwise
  // produce. `pausedRef` mirrors the state for synchronous reads in the scroll
  // handlers; `pausedAtSeqRef` snapshots the arrival counter on the pause edge for the
  // exact "N new" count.
  const [paused, setPaused] = useState(false);
  const pausedRef = useRef(false);
  const lastPauseToggleRef = useRef(0);
  const lastResumeTimeRef = useRef(0);
  const lastNavTimeRef = useRef(0);
  const mountTimeRef = useRef(0);
  const pausedAtSeqRef = useRef(0);
  useEffect(() => {
    mountTimeRef.current = Date.now();
  }, []);
  const scrollPaneToBottom = useCallback(() => {
    requestAnimationFrame(() => {
      const c = paneRef.current?.querySelector('.overflow-y-auto') as HTMLElement | null;
      if (c) c.scrollTo({ top: c.scrollHeight, behavior: 'smooth' });
    });
  }, []);
  const setChatPaused = useCallback(
    (next: boolean, opts?: { force?: boolean; scrollToBottom?: boolean }) => {
      if (pausedRef.current === next) return;
      const now = Date.now();
      if (!opts?.force && now - lastPauseToggleRef.current < PAUSE_SETTLE_MS) return;
      lastPauseToggleRef.current = now;
      pausedRef.current = next;
      if (next) pausedAtSeqRef.current = seqRef.current.next;
      setPaused(next);
      if (!next && opts?.scrollToBottom) {
        lastResumeTimeRef.current = now;
        scrollPaneToBottom();
      }
    },
    [scrollPaneToBottom],
  );

  // Reply jump. Normal panes get this from ChatWidget; the blended pane wires its
  // own, scoped to its own container so it can't grab a same-id row in another
  // surface. Clicking a reply scrolls the merged feed to the quoted message + flashes
  // it (the same `data-message-id` + `.overflow-y-auto` scroll the main pane uses).
  const [highlightedMessageId, setHighlightedMessageId] = useState<string | null>(null);
  const highlightTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(
    () => () => {
      if (highlightTimer.current) clearTimeout(highlightTimer.current);
    },
    [],
  );
  const handleReplyClick = useCallback(
    (parentMsgId: string) => {
      if (!messagesRef.current.some((m) => getMessageId(m) === parentMsgId)) return;
      // Hold the feed (force past the settle window) + mark a navigation so the scroll
      // handlers don't fight the jump; the resume pill takes you back to live.
      lastNavTimeRef.current = Date.now();
      setChatPaused(true, { force: true });
      setHighlightedMessageId(parentMsgId);
      requestAnimationFrame(() => {
        const el = paneRef.current?.querySelector(
          `[data-message-id="${CSS.escape(parentMsgId)}"]`,
        ) as HTMLElement | null;
        const container = el?.closest('.overflow-y-auto') as HTMLElement | null;
        if (el && container) {
          const target = el.offsetTop - container.clientHeight / 2 + el.offsetHeight / 2;
          container.scrollTo({
            top: Math.max(0, Math.min(target, container.scrollHeight - container.clientHeight)),
            behavior: 'smooth',
          });
        }
      });
      if (highlightTimer.current) clearTimeout(highlightTimer.current);
      highlightTimer.current = setTimeout(() => setHighlightedMessageId(null), 2000);
    },
    [getMessageId, setChatPaused],
  );

  // ----- composer -----------------------------------------------------------
  const [text, setText] = useState('');
  const [sending, setSending] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  // From the shared event-driven store. Previously two 5s polls per blended pane,
  // both reading in-memory bools that change only on connect/disconnect.
  const kickConnected = usePlatformAccountStore((s) => s.kick.connected);
  const youtubeConnected = usePlatformAccountStore((s) => s.youtube.connected);
  // Right-click-a-name reply target. The send routes to THIS source + account,
  // overriding the multi-select for that one message.
  const [replyingTo, setReplyingTo] = useState<{ messageId: string; username: string; channel: BlendedChannel } | null>(
    null,
  );
  const inputRef = useRef<HTMLInputElement>(null);
  // Track DESELECTED sources (by sourceKey); everything else is on. New channels
  // are then on by default and closed channels drop out with no reconciliation.
  const [deselected, setDeselected] = useState<Set<string>>(() => new Set());

  const isOn = useCallback((c: BlendedChannel) => !deselected.has(sourceKey(c)), [deselected]);
  const selected = useMemo(() => channels.filter(isOn), [channels, isOn]);

  // Whether we can actually post to a source: Twitch always; Kick/YouTube once
  // their account is connected; TikTok (and other read-only providers) never.
  const canSendTo = useCallback(
    (c: BlendedChannel) => {
      const p = provOf(c);
      if (p === 'twitch') return true;
      if (p === 'kick') return kickConnected;
      if (p === 'youtube') return youtubeConnected;
      return false;
    },
    [kickConnected, youtubeConnected],
  );
  // The per-source picker badge: 'login' (connect to send), 'readonly' (no send
  // path at all), or null (good to go).
  const sendStatus = useCallback(
    (c: BlendedChannel): 'login' | 'readonly' | null => {
      const p = provOf(c);
      if (p === 'twitch') return null;
      if (p === 'kick') return kickConnected ? null : 'login';
      if (p === 'youtube') return youtubeConnected ? null : 'login';
      return 'readonly';
    },
    [kickConnected, youtubeConnected],
  );
  const sendableSelected = useMemo(() => selected.filter(canSendTo), [selected, canSendTo]);

  // Providers present, in canonical order, each with its channels.
  const groups = useMemo(() => {
    return PROVIDER_IDS.map((p) => ({ provider: p, chans: channels.filter((c) => provOf(c) === p) })).filter(
      (g) => g.chans.length > 0,
    );
  }, [channels]);

  const toggleChannel = useCallback((c: BlendedChannel) => {
    const k = sourceKey(c);
    setDeselected((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  }, []);

  const toggleProvider = useCallback(
    (p: ProviderId) => {
      const chans = channels.filter((c) => provOf(c) === p);
      const allOn = chans.every(isOn);
      setDeselected((prev) => {
        const next = new Set(prev);
        // All on -> turn the whole provider off; otherwise turn it fully on.
        for (const c of chans) {
          if (allOn) next.add(sourceKey(c));
          else next.delete(sourceKey(c));
        }
        return next;
      });
    },
    [channels, isOn],
  );

  // Route logins to the Account Connections settings instead of pushing a connect
  // button into the chat space (which shifted the feed). MultiChatWindow listens for
  // this and opens settings on the Connections tab.
  const openConnections = useCallback(() => {
    setPickerOpen(false);
    window.dispatchEvent(new Event('open-multichat-connections'));
  }, []);

  // Right-click a name -> reply to that person, routed to the source they posted in.
  const handleUsernameRightClick = useCallback(
    (messageId: string, username: string) => {
      const channel = idToChannelRef.current.get(messageId);
      if (!channel) return;
      setReplyingTo({ messageId, username, channel });
      requestAnimationFrame(() => inputRef.current?.focus({ preventScroll: true }));
    },
    [],
  );

  // Click a badge on a message -> open its detail in the badges overlay, which lives
  // in the main app (opening main if Go Live closed it).
  const handleBadgeClick = useCallback(
    (badgeKey: string, badgeInfo: { url?: string; image_url_4x?: string }) => {
      const [setId] = badgeKey.split('/');
      void import('../../utils/openBadgesInMain').then(({ openBadgeDetailInMain }) =>
        openBadgeDetailInMain(badgeInfo, setId),
      );
    },
    [],
  );

  // Left-click a name -> open that user's profile card, scoped to the channel they
  // posted in (found via the row's data-message-id) so the card's data AND its
  // Moderator Actions target the right channel — where this mod actually has rights.
  // Twitch-only: the profile card is a Twitch surface. Without this, clicking a name
  // in the blended feed did nothing.
  const handleUsernameClick = useCallback(
    (
      userId: string,
      username: string,
      displayName: string,
      color: string,
      badges: Array<{ key: string; info: { url?: string; image_url_4x?: string } }>,
      event: ReactMouseEvent,
    ) => {
      const row = (event.target as HTMLElement | null)?.closest?.('[data-message-id]');
      const mid = row?.getAttribute('data-message-id') ?? undefined;
      const channel = mid ? idToChannelRef.current.get(mid) : undefined;
      if (channel && provOf(channel) !== 'twitch') return; // no profile/mod surface for other providers here
      const login = channel?.channel;
      // Am I a mod/broadcaster in that channel? My USERSTATE badges live on its slice.
      const badgesStr =
        (login && useChatConnectionStore.getState().channels.get(login.toLowerCase())?.userBadges) || '';
      const isMod = badgesStr.includes('moderator') || badgesStr.includes('broadcaster');
      void openProfilePopup({
        userId,
        username,
        displayName,
        color,
        badges,
        channelName: login,
        isModerator: isMod,
        clientX: event.clientX,
        clientY: event.clientY,
      });
    },
    [],
  );

  // Drop a pending reply if its channel was removed from the blend.
  useEffect(() => {
    if (replyingTo && !channels.some((c) => sourceKey(c) === sourceKey(replyingTo.channel))) {
      setReplyingTo(null);
    }
  }, [channels, replyingTo]);

  const handleSend = useCallback(async () => {
    const body = text.trim();
    if (!body || sending) return;
    // A reply overrides the multi-select: it goes to just the one source the person
    // posted in, through that provider's reply path.
    if (replyingTo) {
      const { channel, messageId, username } = replyingTo;
      // Not logged in to that platform -> can't reply; the connect chip prompts.
      if (!canSendTo(channel)) return;
      setSending(true);
      setText('');
      setReplyingTo(null);
      await sendTo(channel, body, { parentId: messageId, parentUser: username }).catch((e) =>
        Logger.warn(`[Blended] reply to ${sourceKey(channel)} failed:`, e),
      );
      setSending(false);
      return;
    }
    // Only the chats we can actually post to (Twitch, or a connected Kick/YouTube).
    // A selected-but-not-logged-in (or read-only) source is skipped, never silently
    // "sent" — the picker badges + connect chips tell the user why.
    const targets = selected.filter(canSendTo);
    if (targets.length === 0) return;
    setSending(true);
    setText('');
    // Send to each target independently so one failure doesn't block the rest.
    await Promise.all(
      targets.map((c) => sendTo(c, body).catch((e) => Logger.warn(`[Blended] send to ${sourceKey(c)} failed:`, e))),
    );
    setSending(false);
  }, [text, sending, selected, replyingTo, canSendTo]);

  // Scroll-to-pause, mirroring ChatWidget: `onPauseIntent` is the primary pause (fires
  // on a real scroll-up gesture, before any threshold); `onScroll` adds distance-based
  // pause/resume with hysteresis (>150px to pause, <30px to auto-resume) so it can't
  // flap near a single threshold. Grace periods skip the initial layout settle,
  // post-resume inertia, and the reply-jump animation. The resume pill forces past the
  // settle window and glides to the live bottom.
  const inGrace = useCallback(() => {
    const now = Date.now();
    return (
      now - mountTimeRef.current < 2000 ||
      now - lastResumeTimeRef.current < 1000 ||
      now - lastNavTimeRef.current < 1000
    );
  }, []);
  const onPauseIntent = useCallback(() => {
    if (!inGrace()) setChatPaused(true);
  }, [inGrace, setChatPaused]);
  const onScroll = useCallback(
    (distanceToBottom: number, isUserScroll: boolean) => {
      if (inGrace()) return;
      if (isUserScroll && distanceToBottom > 150 && !pausedRef.current) {
        setChatPaused(true);
      } else if (pausedRef.current && distanceToBottom < 30) {
        setChatPaused(false, { scrollToBottom: true });
      }
    },
    [inGrace, setChatPaused],
  );
  const handleResume = useCallback(() => {
    setChatPaused(false, { scrollToBottom: true, force: true });
  }, [setChatPaused]);
  const newSincePause = paused ? Math.max(0, seqRef.current.next - pausedAtSeqRef.current) : 0;

  // Summary label for the picker button.
  const summary =
    selected.length === 0
      ? 'No chats selected'
      : selected.length === channels.length
      ? `All ${channels.length} chat${channels.length > 1 ? 's' : ''}`
      : selected.length === 1
      ? `${selected[0].channelName} · ${PROVIDERS[provOf(selected[0])]?.label ?? provOf(selected[0])}`
      : `${selected.length} of ${channels.length} chats`;

  return (
    <div ref={paneRef} className={`flex h-full min-h-0 min-w-0 flex-1 flex-col ${transparent ? 'bg-transparent' : 'bg-secondary'}`}>
      {[...hypeTrains.values()].map((t) => (
        <div
          key={t.broadcaster_user_login}
          className="flex-shrink-0 border-b border-borderSubtle px-3 pb-2"
        >
          <div className="flex items-center gap-1.5 pt-1.5">
            <ProviderLogo provider="twitch" size={12} className="flex-shrink-0" />
            <span className="truncate text-[11px] font-semibold text-textPrimary">
              {t.broadcaster_user_name || t.broadcaster_user_login}
            </span>
          </div>
          <HypeTrainBanner train={t} confettiTarget={feedEl} />
        </div>
      ))}
      <div ref={setFeedEl} className="relative min-h-0 flex-1 overflow-hidden">
        <ChatMessageList
          messages={shownMessages}
          renderToken={revision}
          isPaused={paused}
          onScroll={onScroll}
          onPauseIntent={onPauseIntent}
          onUsernameClick={handleUsernameClick}
          onReplyClick={handleReplyClick}
          onEmoteRightClick={noop}
          onUsernameRightClick={handleUsernameRightClick}
          onBadgeClick={handleBadgeClick}
          highlightedMessageId={highlightedMessageId}
          deletedMessageIds={deletedMessageIds}
          clearedUserContexts={clearedUserContexts}
          emotes={null}
          getMessageId={getMessageId}
          showSource={channels.length > 1}
        />
        {/* Identical to the core app's paused indicator (ChatWidget) so the resume
            affordance reads the same everywhere. */}
        {paused && (
          <div className="pointer-events-auto absolute bottom-3 left-1/2 z-20 -translate-x-1/2 transform">
            <button
              type="button"
              onClick={handleResume}
              className="flex items-center gap-2 rounded-full px-4 py-2 text-sm font-medium text-white shadow-lg glass-button"
            >
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
              <span>Chat Paused{newSincePause > 0 ? ` (${newSincePause} new)` : ''}</span>
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
            </button>
          </div>
        )}
      </div>

      {!readOnly && (
      <div className="border-t border-white/5 p-2">
        {replyingTo && (
          <div className="mb-2 flex items-center gap-2 rounded-md border border-white/10 bg-white/5 px-2.5 py-1.5">
            <svg className="h-3.5 w-3.5 shrink-0 text-accent" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 10h10a8 8 0 018 8v2M3 10l6 6m-6-6l6-6" />
            </svg>
            <ProviderLogo provider={provOf(replyingTo.channel)} size={13} />
            <span className="min-w-0 flex-1 truncate text-xs text-textSecondary">
              Replying to <span className="font-semibold text-textPrimary">{replyingTo.username}</span>
              <span> on {replyingTo.channel.channelName}</span>
            </span>
            <Tooltip content="Cancel reply">
              <button
                type="button"
                onClick={() => setReplyingTo(null)}
                className="shrink-0 rounded p-0.5 text-textSecondary transition-colors hover:text-textPrimary"
              >
                <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </Tooltip>
          </div>
        )}
        <div className="relative flex items-center gap-2">
          {/* Target picker: themed, grouped-by-provider, multi-select. */}
          <div className="relative shrink-0">
            <Tooltip content="Choose which chats to send to">
              <button
                type="button"
                onClick={() => setPickerOpen((o) => !o)}
                className="glass-input flex items-center gap-1.5 rounded-md px-2.5 py-2 text-xs text-textPrimary"
                style={{ maxWidth: '12rem' }}
              >
                <span className="truncate">{summary}</span>
                <svg
                  className={`h-3 w-3 shrink-0 text-textSecondary transition-transform ${pickerOpen ? 'rotate-180' : ''}`}
                  viewBox="0 0 20 20"
                  fill="currentColor"
                >
                  <path
                    fillRule="evenodd"
                    d="M5.3 7.3a1 1 0 011.4 0L10 10.6l3.3-3.3a1 1 0 111.4 1.4l-4 4a1 1 0 01-1.4 0l-4-4a1 1 0 010-1.4z"
                    clipRule="evenodd"
                  />
                </svg>
              </button>
            </Tooltip>

            {pickerOpen && (
              <>
                {/* click-away backdrop */}
                <div className="fixed inset-0 z-40" onClick={() => setPickerOpen(false)} />
                <div
                  className="glass-panel absolute bottom-full z-50 mb-1 max-h-64 w-56 overflow-y-auto rounded-lg border border-borderLight py-1 shadow-2xl scrollbar-thin"
                  // Opaque themed surface: a live backdrop-blur flickers over chat.
                  style={{ backgroundColor: 'var(--color-background-tertiary)', backdropFilter: 'none', WebkitBackdropFilter: 'none' }}
                >
                  {groups.map((g) => {
                    const allOn = g.chans.every(isOn);
                    const someOn = g.chans.some(isOn);
                    return (
                      <div key={g.provider}>
                        <button
                          type="button"
                          onClick={() => toggleProvider(g.provider)}
                          className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-xs font-semibold text-textPrimary transition-colors hover:bg-white/5"
                        >
                          <Check checked={allOn} indeterminate={!allOn && someOn} />
                          <ProviderLogo provider={g.provider} size={14} />
                          <span>{PROVIDERS[g.provider]?.label ?? g.provider}</span>
                        </button>
                        {g.chans.map((c) => {
                          const status = sendStatus(c);
                          return (
                            <div
                              key={sourceKey(c)}
                              className="flex w-full items-center gap-2 py-1.5 pl-8 pr-2.5 text-xs transition-colors hover:bg-white/5"
                            >
                              <button
                                type="button"
                                onClick={() => toggleChannel(c)}
                                className="flex min-w-0 flex-1 items-center gap-2 text-left text-textSecondary transition-colors hover:text-textPrimary"
                              >
                                <Check checked={isOn(c)} />
                                <span className="min-w-0 flex-1 truncate">{c.channelName}</span>
                              </button>
                              {status === 'login' && (
                                <Tooltip content="Sign in from Account Connections">
                                  <button
                                    type="button"
                                    onClick={openConnections}
                                    className="shrink-0 rounded bg-amber-500/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-amber-400 transition-colors hover:bg-amber-500/30"
                                  >
                                    Log in
                                  </button>
                                </Tooltip>
                              )}
                              {status === 'readonly' && (
                                <span className="shrink-0 rounded bg-white/10 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-textMuted">
                                  Read-only
                                </span>
                              )}
                            </div>
                          );
                        })}
                      </div>
                    );
                  })}
                </div>
              </>
            )}
          </div>

          <input
            ref={inputRef}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                void handleSend();
              } else if (e.key === 'Escape' && replyingTo) {
                e.preventDefault();
                setReplyingTo(null);
              }
            }}
            placeholder={
              replyingTo
                ? canSendTo(replyingTo.channel)
                  ? `Reply to ${replyingTo.username}…`
                  : `Log in to reply on ${PROVIDERS[provOf(replyingTo.channel)]?.label ?? provOf(replyingTo.channel)}…`
                : sendableSelected.length === 0
                ? 'Log in to send to these chats…'
                : sendableSelected.length < selected.length
                ? `Send to ${sendableSelected.length} connected chat${sendableSelected.length > 1 ? 's' : ''}…`
                : selected.length > 1
                ? 'Send to selected chats…'
                : 'Send a message…'
            }
            className="glass-input min-w-0 flex-1 rounded-md px-3 py-2 text-sm text-textPrimary placeholder-textSecondary focus:outline-none"
          />
          <Tooltip content="Send">
            <button
              type="button"
              onClick={() => void handleSend()}
              disabled={
                !text.trim() ||
                sending ||
                (replyingTo ? !canSendTo(replyingTo.channel) : sendableSelected.length === 0)
              }
              className="glass-button flex h-9 w-9 shrink-0 items-center justify-center self-center rounded text-white transition-all disabled:cursor-not-allowed disabled:opacity-50"
            >
              <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" />
              </svg>
            </button>
          </Tooltip>
        </div>
      </div>
      )}
    </div>
  );
}

export default BlendedChatPane;
