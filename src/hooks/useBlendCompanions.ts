// The streamer's OTHER platforms, attached to the chat panel so their messages
// can be merged into the one you are watching.
//
// Off by default and free when off: with blend disabled this asks Rust nothing,
// opens no connection, and holds no state. Nothing here polls — a companion's
// liveness is discovered by attaching to it, not by a separate check, so there
// is no second network call and no snapshot to go stale.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { acquireChannel, releaseChannel } from '../stores/chatConnectionStore';
import { useAppStore } from '../stores/AppStore';
import { makeKey } from '../utils/providerKey';
import { Logger } from '../utils/logger';
import type { ProviderId } from '../types/providers';
import type { ChannelLinkGroup, LinkMember } from '../types';

export interface BlendCompanion {
  provider: ProviderId;
  /** Verbatim from the link record — never rebuilt from a slice key, because a
   *  YouTube channel id is case-sensitive and slice keys are lowercased. */
  channel: string;
  channelName: string;
}

interface ChannelLinkView {
  group: ChannelLinkGroup | null;
  companions: LinkMember[];
}

/** A channel that might be this streamer somewhere else. Never linked without
 *  being accepted: a same-named stranger is a real possibility, and their chat
 *  arriving under this streamer's name is worse than no suggestion. */
export interface LinkSuggestion {
  provider: ProviderId;
  channel: string;
  candidate: LinkMember;
  title: string;
  is_live: boolean;
}

/** Long enough that flicking through channels probes none of them, short enough
 *  that settling on one offers the link while you are still looking at it. */
const PROBE_DELAY_MS = 6000;

const EMPTY: BlendCompanion[] = [];

const toCompanion = (m: LinkMember): BlendCompanion => ({
  provider: m.provider,
  channel: m.channel,
  channelName: m.display_name || m.channel,
});

export function useBlendCompanions(provider: ProviderId, channel: string | null) {
  const blend = useAppStore((s) => s.settings.chat_blend);
  const enabled = blend?.enabled === true;
  const platforms = blend?.platforms;

  // Stamped with the channel it was fetched FOR, so switching channels can never
  // briefly show the previous streamer's companions while the next read is in
  // flight. Deriving `linked` from it also keeps the disabled case out of state
  // entirely, rather than writing an empty list on every render that it is off.
  const [fetched, setFetched] = useState<{ key: string; list: BlendCompanion[] }>({
    key: '',
    list: EMPTY,
  });
  const wantKey = enabled && channel ? makeKey(provider, channel) : '';
  const linked = wantKey && fetched.key === wantKey ? fetched.list : EMPTY;

  // Re-read on demand (after accepting or removing a link) without giving the
  // caller a function whose identity changes.
  const [reloadNonce, setReloadNonce] = useState(0);
  const refresh = useCallback(() => setReloadNonce((n) => n + 1), []);

  // Ask Rust which channels belong to this streamer. Only when the feature is
  // on: a disabled blend must not invoke anything on every channel you open.
  useEffect(() => {
    if (!wantKey || !channel) return;
    let cancelled = false;
    void (async () => {
      try {
        const view = await invoke<ChannelLinkView>('get_channel_links', { provider, channel });
        if (cancelled) return;
        setFetched({
          key: wantKey,
          list: view.companions.length ? view.companions.map(toCompanion) : EMPTY,
        });
      } catch (err) {
        Logger.warn('[Blend] could not read channel links:', err);
        if (!cancelled) setFetched({ key: wantKey, list: EMPTY });
      }
    })();
    // A read for the channel you just left must not land on the one you are now
    // watching. `wantKey` already gates what renders; this stops the pointless
    // write as well.
    return () => {
      cancelled = true;
    };
  }, [wantKey, provider, channel, reloadNonce]);

  // Ask whether this streamer exists on another platform, once the viewer has
  // actually settled on the channel. Kick only, and only while the feature is
  // on; Rust holds the negative cache, so reopening a channel asks nothing.
  // Derived against the channel on screen rather than cleared on change, so the
  // effect below writes no state synchronously and a late-arriving probe for the
  // previous channel can never surface here.
  const [offered, setOffered] = useState<LinkSuggestion | null>(null);
  const suggestion =
    offered && wantKey && makeKey(offered.provider, offered.channel) === wantKey ? offered : null;
  useEffect(() => {
    if (!wantKey || !channel || blend?.suggest_links === false) return;
    const t = window.setTimeout(() => {
      void invoke('probe_channel_links', { provider, channel }).catch((err) =>
        Logger.warn('[Blend] probe failed:', err),
      );
    }, PROBE_DELAY_MS);
    return () => window.clearTimeout(t);
  }, [wantKey, provider, channel, blend?.suggest_links]);

  useEffect(() => {
    if (!wantKey) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<LinkSuggestion>('channel-links', (e) => {
      // A probe that lands after the viewer has moved on is for a channel that
      // is no longer on screen, so it is dropped rather than offered here.
      const s = e.payload;
      if (makeKey(s.provider, s.channel) !== wantKey) return;
      setOffered(s);
    })
      .then((un) => {
        if (cancelled) un();
        else unlisten = un;
      })
      .catch((err) => Logger.warn('[Blend] could not listen for link suggestions:', err));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [wantKey]);

  // Which of them the user actually wants in the feed. A platform absent from
  // the map is allowed; only an explicit false excludes it.
  //
  // Memoized because this list is a dependency of the merge, and the merge walks
  // every open chat slice. A fresh array each render would re-run that walk on
  // every render of the chat panel, including every keystroke in its composer.
  const attached = useMemo(
    () => (enabled ? linked.filter((c) => platforms?.[c.provider] !== false) : EMPTY),
    [enabled, linked, platforms],
  );

  // Hold a chat connection per attached companion, ref-counted by the store so a
  // channel also open in MultiChat survives when this releases it. The desired /
  // held diff is idempotent, so StrictMode's double-invoke is a no-op.
  const heldRef = useRef<Map<string, BlendCompanion>>(new Map());
  const attachedKey = attached.map((c) => makeKey(c.provider, c.channel)).join('|');
  useEffect(() => {
    const desired = new Map(attached.map((c) => [makeKey(c.provider, c.channel), c]));
    for (const [key, want] of desired) {
      if (heldRef.current.has(key)) continue;
      heldRef.current.set(key, want);
      // `channelId` is null on purpose: each adapter resolves its own (Kick's
      // chatroom id from the slug, YouTube's live video from the channel id), and
      // guessing one here would be a second resolver to drift from theirs.
      void acquireChannel(want.channel, null, want.provider).catch((err) =>
        Logger.error(`[Blend] could not attach ${key}:`, err),
      );
    }
    for (const [key, held] of Array.from(heldRef.current)) {
      if (desired.has(key)) continue;
      heldRef.current.delete(key);
      void releaseChannel(held.channel, held.provider).catch((err) =>
        Logger.warn(`[Blend] could not release ${key}:`, err),
      );
    }
    // Keyed on the joined list rather than the array, whose identity changes on
    // every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [attachedKey]);

  // Let go of everything when the panel goes away, so a companion's connection
  // (and its 1-3 MB of emote metadata) never outlives the chat that opened it.
  useEffect(
    () => () => {
      for (const [, held] of Array.from(heldRef.current)) {
        void releaseChannel(held.channel, held.provider).catch(() => {});
      }
      heldRef.current.clear();
    },
    [],
  );

  return { linked, attached, refresh, enabled, suggestion, dismissSuggestion: () => setOffered(null) };
}
