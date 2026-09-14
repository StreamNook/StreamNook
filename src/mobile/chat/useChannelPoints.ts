// Channel points: the balance, the channel's own points branding, the bonus
// chest, and the credit feed the composer animates.
//
// `get_channel_points_for_channel` runs on the drops token, so this quietly
// yields nothing until the drops account is connected. That is the right
// failure mode: the composer just does not show a points button rather than
// showing a broken one.
//
// The response nests under either `data.community.channel` or
// `data.user.channel` depending on which web client answered, so both are
// accepted, same rule the desktop composer uses.
//
// Two things this has to do that are not obvious.
//
// The bonus chest is collected by the FRONTEND, not by Rust. There is no
// backend poller and no backend auto-claim for the channel you are watching;
// the drops service says so outright, and the only background sweep lives in
// the automation plugin, which does not run here. So if this hook does not
// claim the chest, nothing does, and every chest on the phone expires.
//
// Passive per-minute points DO get credited already, by the Rust watch
// heartbeat, but nothing announces them. There is no event and no push. The
// only way to see that earning is happening is to ask again and compare, which
// is what the poll below is for: the difference between two reads is the credit
// that just landed.
import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import { isBackgrounded } from '../backgroundGate';

// Matches the desktop composer's cadence. Fast enough that a chest is collected
// well inside its lifetime and the balance never looks frozen, slow enough to
// be nothing on a phone's battery.
const POLL_MS = 60_000;

// Matches the desktop composer's retry on the same query: three tries, a second
// apart. One bad read on a phone should not empty the panel for a whole minute.
const READ_ATTEMPTS = 3;
const READ_RETRY_MS = 1000;

export interface ChannelPointsState {
  balance: number | null;
  /** The channel's own name for its points, e.g. "Bones". */
  name: string | null;
  /** The channel's custom points icon; null means render the default glyph. */
  iconUrl: string | null;
  /** A chest waiting to be collected, when auto-collect is turned off. */
  availableClaimId: string | null;
  /** The most recent credit, for the composer's floating "+N". Carries an id so
   *  two identical amounts in a row still read as two separate credits. */
  gain: { id: number; amount: number } | null;
  clearGain: () => void;
  /** Collect the chest by hand. No-op when there is nothing to collect. */
  claimChest: () => void;
  refresh: () => void;
}

interface Loaded {
  /** Which channel this result belongs to; anything else is stale. */
  login: string;
  balance: number | null;
  name: string | null;
  iconUrl: string | null;
  claimId: string | null;
}

export function useChannelPoints(
  channelLogin: string | null | undefined,
  channelId?: string | null,
): ChannelPointsState {
  // Stamped with its channel rather than cleared on switch, so changing streams
  // cannot briefly show the previous channel's balance and nothing has to call
  // setState synchronously in the effect to prevent that.
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [gain, setGain] = useState<{ id: number; amount: number } | null>(null);
  const [nonce, setNonce] = useState(0);

  const autoClaim = useAppStore((s) => s.settings.auto_claim_points_watching ?? true);

  // Last balance seen for a given channel, so a credit can be spotted as the
  // difference between two reads. Keyed by channel because switching rooms
  // changes the balance for reasons that are not earnings.
  const lastBalance = useRef<{ login: string; balance: number } | null>(null);
  const gainSeq = useRef(0);
  const claiming = useRef(false);

  const refresh = useCallback(() => setNonce((n) => n + 1), []);
  const clearGain = useCallback(() => setGain(null), []);

  // Single choke point for every credit, from either source, so a chest can
  // never be reported twice.
  //
  // Float only, no system notification. The old hidden-only notification could
  // in practice fire ONLY in picture-in-picture (the one state that is
  // `hidden` while this poll still runs), where it flooded the shade with a
  // "+N" every poll. Foreground has the float, and a truly backgrounded shell
  // never reaches this code; honest background points alerts would need the
  // Rust watch heartbeat to report credits itself.
  const noteGain = useCallback((amount: number) => {
    if (amount <= 0) return;
    gainSeq.current += 1;
    setGain({ id: gainSeq.current, amount });
  }, []);

  // Collecting the chest. The command returns the exact credited amount with
  // multipliers applied, which is better than a balance delta. Guarded against
  // overlapping runs because the poll and a tap can both reach it.
  //
  // Deliberately does NOT record the lifetime points stat. This command emits
  // `channel-points-earned` on its way out, and the boot listener already counts
  // that event; doing it here as well would score every chest twice.
  const claim = useCallback(
    async (claimId: string, id: string, login: string): Promise<number> => {
      if (claiming.current) return 0;
      claiming.current = true;
      try {
        const result = await invoke<{ new_balance: number; points_earned: number }>(
          'claim_channel_points',
          { channelId: id, channelName: login, claimId },
        );
        return result.points_earned > 0 ? result.points_earned : 0;
      } catch (err) {
        Logger.warn('[MobilePoints] chest claim failed:', err);
        return 0;
      } finally {
        claiming.current = false;
      }
    },
    [],
  );

  useEffect(() => {
    if (!channelLogin) return;
    let cancelled = false;

    const read = async (attempt = 0) => {
      try {
        const result = await invoke<unknown>('get_channel_points_for_channel', { channelLogin });
        if (cancelled) return;
        const data = (result as { data?: Record<string, { channel?: unknown }> })?.data;
        const channel = (data?.community?.channel ?? data?.user?.channel) as
          | {
              self?: { communityPoints?: { balance?: unknown; availableClaim?: { id?: unknown } | null } };
              communityPointsSettings?: { name?: unknown; image?: { url?: unknown } };
            }
          | undefined;

        const community = channel?.self?.communityPoints;
        const rawBalance = community?.balance;
        const balance = typeof rawBalance === 'number' ? rawBalance : null;
        const settings = channel?.communityPointsSettings;
        const rawClaim = community?.availableClaim?.id;
        let claimId = typeof rawClaim === 'string' ? rawClaim : null;

        // A credit since the last look. This is what makes passive earning
        // visible: the watch heartbeat has been crediting points all along with
        // nothing to show for it.
        const prev = lastBalance.current;
        if (balance !== null && prev && prev.login === channelLogin && balance > prev.balance) {
          noteGain(balance - prev.balance);
        }
        if (balance !== null) lastBalance.current = { login: channelLogin, balance };

        if (claimId && channelId && autoClaim) {
          const earned = await claim(claimId, channelId, channelLogin);
          if (cancelled) return;
          claimId = null;
          if (earned > 0) {
            noteGain(earned);
            // The claim moved the balance past what was just read; keep the
            // baseline in step or the next poll reports the same points twice.
            const base = lastBalance.current;
            if (base && base.login === channelLogin) {
              lastBalance.current = { login: channelLogin, balance: base.balance + earned };
            }
          }
        }

        setLoaded({
          login: channelLogin,
          balance,
          name: typeof settings?.name === 'string' ? settings.name : null,
          iconUrl: typeof settings?.image?.url === 'string' ? settings.image.url : null,
          claimId,
        });
      } catch (err) {
        if (cancelled) return;
        // Not connected, or the channel has points off. Both are normal.
        //
        // Retry before blanking, the way the desktop composer does. A single
        // failed read on a phone is usually a moment of bad signal, and giving
        // up on it drops the points button out of the composer until the next
        // poll a minute later.
        if (attempt < READ_ATTEMPTS - 1) {
          await new Promise((resolve) => window.setTimeout(resolve, READ_RETRY_MS));
          if (cancelled) return;
          await read(attempt + 1);
          return;
        }
        Logger.debug('[MobilePoints] balance unavailable:', err);
        setLoaded({ login: channelLogin, balance: null, name: null, iconUrl: null, claimId: null });
      }
    };

    void read();
    // Chests appear mid-stream and passive points land every few minutes, so a
    // single read at join time goes stale within a minute of arriving.
    const timer = window.setInterval(() => {
      // Skipped while backgrounded. The balance is re-read on resume, and the
      // chest claim this drives needs the app in front to be worth anything.
      if (isBackgrounded()) return;
      void read();
    }, POLL_MS);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [channelLogin, channelId, nonce, autoClaim, claim, noteGain]);

  const fresh = channelLogin && loaded?.login === channelLogin ? loaded : null;

  const claimChest = useCallback(() => {
    if (!fresh?.claimId || !channelId || !channelLogin) return;
    void (async () => {
      const earned = await claim(fresh.claimId!, channelId, channelLogin);
      if (earned > 0) {
        noteGain(earned);
        // Same baseline bump as the auto path: refresh() re-reads the balance,
        // and without moving the baseline past the claim first, that read sees
        // the claimed amount as a fresh delta and floats it a second time.
        const base = lastBalance.current;
        if (base && base.login === channelLogin) {
          lastBalance.current = { login: channelLogin, balance: base.balance + earned };
        }
      }
      refresh();
    })();
  }, [fresh?.claimId, channelId, channelLogin, claim, noteGain, refresh]);

  return {
    balance: fresh?.balance ?? null,
    name: fresh?.name ?? null,
    iconUrl: fresh?.iconUrl ?? null,
    availableClaimId: fresh?.claimId ?? null,
    gain,
    clearGain,
    claimChest,
    refresh,
  };
}
