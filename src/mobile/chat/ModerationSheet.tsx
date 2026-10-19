// The room's moderation in one place, from the chat menu's "Moderation" row:
// what AutoMod is holding (Allow or Deny), what moderators did lately (with
// Unban on a ban or timeout), blocking a phrase, stopping a running /nuke, and
// the way into Manage channel for chat modes and Shield Mode. The same sheet
// as the iPhone's. Rust keeps the held queue (`get_automod_queue`) and the room
// log (`get_room_mod_log`, pushed as `room-mod-log`); every action goes
// through the core.
import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { CaretRight, Flag, SlidersHorizontal, StopCircle } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { useAutomodStore, type HeldMessage } from '../../stores/automodStore';
import { moderateChatter } from '../../services/moderation';
import { Logger } from '../../utils/logger';

export interface RoomModAction {
  id: string;
  channel: string;
  action: string;
  moderator: string;
  target: string | null;
  target_id: string | null;
  line: string;
  quote: string | null;
  at_ms: number;
  liftable: boolean;
}

interface Props {
  open: boolean;
  onClose: () => void;
  channelId: string;
  channelLogin: string;
  onManageChannel: () => void;
}

const AGO = new Intl.RelativeTimeFormat([], { numeric: 'auto', style: 'short' });

/** "2 min. ago", the phone's own words for how long ago. */
function ago(atMs: number): string {
  const secs = Math.round((atMs - Date.now()) / 1000);
  const abs = Math.abs(secs);
  if (abs < 60) return AGO.format(secs, 'second');
  if (abs < 3600) return AGO.format(Math.round(secs / 60), 'minute');
  if (abs < 86400) return AGO.format(Math.round(secs / 3600), 'hour');
  return AGO.format(Math.round(secs / 86400), 'day');
}

const words = (err: unknown) => (typeof err === 'string' ? err : 'That did not go through');

export const ModerationSheet: React.FC<Props> = ({ open, onClose, channelId, channelLogin, onManageChannel }) => {
  const channel = channelLogin.toLowerCase();
  const held = useAutomodStore((s) => s.held[channel]);
  const [log, setLog] = useState<RoomModAction[] | null>(null);
  const [nukes, setNukes] = useState(0);
  const [busy, setBusy] = useState<string | null>(null);
  const [heldNote, setHeldNote] = useState<string | null>(null);
  const [logNote, setLogNote] = useState<string | null>(null);
  const [phrase, setPhrase] = useState('');
  const [phraseNote, setPhraseNote] = useState<string | null>(null);

  const reload = useCallback(async () => {
    void useAutomodStore.getState().seed(channel);
    try {
      setLog(await invoke<RoomModAction[]>('get_room_mod_log', { channel }));
      setNukes(await invoke<number>('nuke_active', { channel }));
    } catch (err) {
      Logger.debug('[Moderation] room log unavailable:', err);
      setLog([]);
    }
  }, [channel]);

  useEffect(() => {
    if (!open) return;
    void reload();
    // Rust pushes each moderator action in the room as it happens.
    const off = listen<RoomModAction>('room-mod-log', (e) => {
      const entry = e.payload;
      if (entry.channel !== channel) return;
      setLog((cur) => (cur?.some((x) => x.id === entry.id) ? cur : [entry, ...(cur ?? [])]));
    });
    return () => {
      void off.then((f) => f());
    };
  }, [open, channel, reload]);

  const decide = async (message: HeldMessage, allow: boolean) => {
    setBusy(message.message_id);
    const failure = await useAutomodStore.getState().resolve(message.message_id, allow);
    setBusy(null);
    setHeldNote(failure);
  };

  const lift = async (entry: RoomModAction) => {
    if (!entry.target_id) return;
    setBusy(entry.id);
    try {
      const outcome = await moderateChatter({
        provider: 'twitch',
        broadcasterId: channelId,
        targetUserId: entry.target_id,
        targetName: entry.target,
        action: { kind: 'unban', timeout: entry.action === 'timeout' },
      });
      setLogNote(outcome.message);
    } catch (err) {
      setLogNote(words(err));
    } finally {
      setBusy(null);
    }
  };

  const blockPhrase = async () => {
    const text = phrase.trim();
    if (!text) return;
    setBusy('phrase');
    try {
      await invoke('add_blocked_term', { broadcasterId: channelId, text });
      setPhraseNote(`Blocked “${text}”`);
      setPhrase('');
    } catch (err) {
      setPhraseNote(words(err));
    } finally {
      setBusy(null);
    }
  };

  const stopNukes = async () => {
    await invoke('nuke_stop', { channel }).catch(() => {});
    setNukes(0);
  };

  const heading = 'px-1 pb-1.5 pt-4 text-[12px] font-semibold uppercase tracking-[0.05em] text-textMuted';
  const note = 'px-1 pt-1.5 text-[12px] text-textSecondary';

  return (
    <MobileSheet open={open} onClose={onClose} title="Moderation" maxHeightFraction={0.85}>
      <div className="px-3 pb-4">
        <div className={heading}>{held && held.length > 0 ? `Held by AutoMod · ${held.length}` : 'Held by AutoMod'}</div>
        <div className="glass-panel divide-y divide-white/[0.06] overflow-hidden">
          {!held || held.length === 0 ? (
            <p className="px-3 py-3 text-[13px] text-textMuted">Nothing held right now.</p>
          ) : (
            held.map((m) => (
              <div key={m.message_id} className="px-3 py-2.5">
                <div className="flex items-center gap-1.5 text-[12px] font-semibold text-warning">
                  <Flag size={12} weight="fill" />
                  {m.label ?? 'Held by AutoMod'}
                </div>
                <p className="mt-1 text-[14px] leading-snug text-textPrimary [overflow-wrap:anywhere]">
                  <span className="font-bold">{m.user_name || m.user_login}</span>: {m.text}
                </p>
                <div className="mt-2 flex gap-2">
                  <button
                    type="button"
                    disabled={busy != null}
                    onClick={() => void decide(m, true)}
                    className="h-[34px] flex-1 rounded-[10px] bg-success/15 text-[14px] font-semibold text-success disabled:opacity-60"
                  >
                    Allow
                  </button>
                  <button
                    type="button"
                    disabled={busy != null}
                    onClick={() => void decide(m, false)}
                    className="h-[34px] flex-1 rounded-[10px] bg-error/15 text-[14px] font-semibold text-error disabled:opacity-60"
                  >
                    Deny
                  </button>
                </div>
              </div>
            ))
          )}
        </div>
        {heldNote && <p className={note}>{heldNote}</p>}

        <div className={heading}>Recent actions</div>
        <div className="glass-panel divide-y divide-white/[0.06] overflow-hidden">
          {log == null ? null : log.length === 0 ? (
            <p className="px-3 py-3 text-[13px] text-textMuted">No moderator actions since you opened this chat.</p>
          ) : (
            log.map((e) => (
              <div key={e.id} className="flex items-start gap-2.5 px-3 py-2.5">
                <div className="min-w-0 flex-1">
                  <div className="text-[14px] text-textPrimary">{e.line}</div>
                  {e.quote && (
                    <div className="mt-0.5 text-[13px] leading-snug text-textSecondary [overflow-wrap:anywhere]">“{e.quote}”</div>
                  )}
                  <div className="mt-0.5 text-[12px] text-textMuted">{ago(e.at_ms)}</div>
                </div>
                {e.liftable && e.target_id && (
                  <button
                    type="button"
                    disabled={busy != null}
                    onClick={() => void lift(e)}
                    className="shrink-0 px-1 py-1 text-[13px] font-semibold text-accent disabled:opacity-50"
                  >
                    {e.action === 'timeout' ? 'Lift' : 'Unban'}
                  </button>
                )}
              </div>
            ))
          )}
        </div>
        {logNote && <p className={note}>{logNote}</p>}

        <div className={heading}>Room</div>
        <div className="glass-panel divide-y divide-white/[0.06] overflow-hidden">
          <div className="flex items-center gap-2 px-3 py-1.5">
            <input
              value={phrase}
              onChange={(e) => setPhrase(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && void blockPhrase()}
              placeholder="Block a phrase"
              enterKeyHint="done"
              autoCapitalize="off"
              autoCorrect="off"
              className="h-9 min-w-0 flex-1 bg-transparent text-[15px] text-textPrimary outline-none placeholder:text-textMuted"
            />
            <button
              type="button"
              disabled={!phrase.trim() || busy != null}
              onClick={() => void blockPhrase()}
              className="shrink-0 px-1 text-[14px] font-semibold text-accent disabled:opacity-40"
            >
              Block
            </button>
          </div>
          {nukes > 0 && (
            <button type="button" onClick={() => void stopNukes()} className="flex h-11 w-full items-center gap-2.5 px-3 text-left text-[14px] text-error">
              <StopCircle size={18} />
              Stop the running /nuke
            </button>
          )}
          <button
            type="button"
            onClick={() => {
              onClose();
              onManageChannel();
            }}
            className="flex h-11 w-full items-center gap-2.5 px-3 text-left text-[14px] text-textPrimary"
          >
            <SlidersHorizontal size={18} className="text-textSecondary" />
            <span className="flex-1">Chat modes and Shield Mode</span>
            <CaretRight size={16} className="text-textMuted" />
          </button>
        </div>
        {phraseNote && <p className={note}>{phraseNote}</p>}
      </div>
    </MobileSheet>
  );
};
