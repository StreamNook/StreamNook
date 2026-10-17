// Channel points and the wallet's bits face for one Twitch channel on the
// phone: the balance, the channel's own points branding, the bonus chest, and
// the "+N" the composer animates.
//
// Everything is read from Rust's channel_state (the same watch the player and
// the desktop composer use), so the phone no longer runs its own 60 s poll of
// the same channel. Rust polls points every minute, applies every gain the
// points socket reports, and records the last one (`last_gain`), which is the
// single source for the "+N": watching, the chest, predictions, a rise only
// the poll saw. Bits come from the same state (`bits`).
//
// `get_channel_points_for_channel` runs on the drops token, so this quietly
// yields nothing until the drops account is connected. That is the right
// failure mode: the composer just does not show a points button rather than
// showing a broken one.
//
// The bonus chest is still collected HERE when auto-collect is on. Rust's own
// collector only takes chests on channels the watch session registered as
// active, which the phone's player does not do, so without this every chest
// on the phone would expire. Rust turns a second claim on the same chest away,
// so this and Rust can never collect one twice.
import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { refreshChannelState, unwatchChannel, useChannelState, watchChannel } from '../../stores/channelStateStore';
import type { ChannelBitsFace } from '../../types';
import { Logger } from '../../utils/logger';

export interface ChannelPointsState {
  balance: number | null;
  /** The channel's own name for its points, e.g. "Bones". */
  name: string | null;
  /** The channel's custom points icon; null means render the default glyph. */
  iconUrl: string | null;
  /** A chest waiting to be collected, when auto-collect is turned off. */
  availableClaimId: string | null;
  /** The bits face, when the channel has bits on. */
  bits: ChannelBitsFace | null;
  /** The most recent credit, for the composer's floating "+N". Carries an id so
   *  two identical amounts in a row still read as two separate credits. */
  gain: { id: number; amount: number } | null;
  clearGain: () => void;
  /** Collect the chest by hand. No-op when there is nothing to collect. */
  claimChest: () => void;
  refresh: () => void;
}

export function useChannelPoints(
  channelLogin: string | null | undefined,
  channelId?: string | null,
): ChannelPointsState {
  const login = channelLogin ? channelLogin.toLowerCase() : null;
  const state = useChannelState(login);
  useEffect(() => {
    if (!login || !channelId) return;
    void watchChannel(login, channelId);
    return () => {
      void unwatchChannel(login);
    };
  }, [login, channelId]);

  const autoClaim = useAppStore((s) => s.settings.auto_claim_points_watching ?? true);
  const chestOnly = useAppStore((s) => s.settings.chat_input?.wallet_gains === 'chest');
  const points = state?.points?.enabled ? state.points : null;
  const bits = state?.bits?.enabled ? state.bits : null;

  // A new gain from Rust becomes a "+N". The first points answer for a
  // channel is the baseline, so a gain recorded before it never plays.
  const lastGain = points?.last_gain ?? null;
  const [seen, setSeen] = useState<{ login: string | null; seq: number | null }>({ login, seq: null });
  const [gain, setGain] = useState<{ id: number; amount: number } | null>(null);
  const seq = lastGain?.seq ?? 0;
  if (points && (seen.login !== login || seen.seq === null)) {
    setSeen({ login, seq });
  } else if (points && seen.seq !== null && seq > seen.seq) {
    setSeen({ login, seq });
    if (lastGain && (!chestOnly || lastGain.chest)) setGain({ id: seq, amount: lastGain.amount });
  }
  const clearGain = useCallback(() => setGain(null), []);

  const claiming = useRef(false);
  const claim = useCallback(
    async (claimId: string) => {
      if (claiming.current || !login || !channelId) return;
      claiming.current = true;
      try {
        await invoke('claim_channel_points', { channelId, channelName: login, claimId });
      } catch (err) {
        Logger.warn('[MobilePoints] chest claim failed:', err);
      } finally {
        claiming.current = false;
      }
    },
    [login, channelId],
  );

  // Auto-collect: the chest Rust's poll (or the socket) just reported.
  const claimId = points?.available_claim_id ?? null;
  useEffect(() => {
    if (claimId && autoClaim) void claim(claimId);
  }, [claimId, autoClaim, claim]);

  const refresh = useCallback(() => {
    if (login) void refreshChannelState(login, 'points');
  }, [login]);
  const claimChest = useCallback(() => {
    if (!claimId) return;
    void claim(claimId).then(refresh);
  }, [claimId, claim, refresh]);

  return {
    balance: points?.balance ?? null,
    name: points?.name ?? null,
    iconUrl: points?.icon_url ?? null,
    availableClaimId: autoClaim ? null : claimId,
    bits,
    gain,
    clearGain,
    claimChest,
    refresh,
  };
}
