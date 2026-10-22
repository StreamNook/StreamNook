// The room's moderation in one place, from the chat menu's "Moderation" row:
// what AutoMod is holding (Allow or Deny), who is asking to be unbanned
// (Approve or Deny, with an optional reply), what moderators did lately (with
// Unban on a ban or timeout), blocking a phrase, stopping a running /nuke, and
// the way into Manage channel for chat modes and Shield Mode. The same sheet
// as the iPhone's. Rust keeps the held queue (`get_automod_queue`) and the room
// log (`get_room_mod_log`, pushed as `room-mod-log`); every action goes
// through the core.
//
// Nothing moves under a thumb: actions arriving while the sheet is open wait
// behind an "N new" pill, a decided held message says Allowed or Denied for a
// moment before it goes, and Unban asks once before it runs.
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ArrowUp, CaretRight, Flag, SlidersHorizontal, StopCircle } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { useAutomodStore, type HeldMessage } from '../../stores/automodStore';
import { UNBAN_REPLY_MAX, useUnbanRequests, type UnbanRequest } from '../../components/chat/unbanRequests';
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

/** How long a decided held message stays, saying what happened. */
const DECIDED_MS = 1500;

interface Decided {
  message: HeldMessage;
  outcome: 'Allowed' | 'Denied';
  index: number;
}

interface DecidedUnban {
  request: UnbanRequest;
  outcome: 'Approved' | 'Denied';
  index: number;
}

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
  /** The phrase just blocked, while its Undo is offered. */
  const [blocked, setBlocked] = useState<string | null>(null);
  /** Actions that arrived while the sheet is open, behind the "N new" pill. */
  const [incoming, setIncoming] = useState<RoomModAction[]>([]);
  const [decided, setDecided] = useState<Record<string, Decided>>({});
  /** The Unban or Lift waiting for its confirmation. */
  const [liftPending, setLiftPending] = useState<string | null>(null);
  const { rows: unbanRequests, resolve: resolveUnban } = useUnbanRequests(channelId, channel, open && !!channelId);
  const [unbanNote, setUnbanNote] = useState<string | null>(null);
  const [unbanReplies, setUnbanReplies] = useState<Record<string, string>>({});
  const [unbanDecided, setUnbanDecided] = useState<Record<string, DecidedUnban>>({});

  const reload = useCallback(async () => {
    setIncoming([]);
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
      setIncoming((cur) => (cur.some((x) => x.id === entry.id) ? cur : [entry, ...cur]));
    });
    return () => {
      void off.then((f) => f());
    };
  }, [open, channel, reload]);

  const showIncoming = () => {
    setLog((cur) => [...incoming.filter((x) => !cur?.some((y) => y.id === x.id)), ...(cur ?? [])]);
    setIncoming([]);
  };

  // A decided message keeps its place, saying what happened, until it goes.
  const heldRows = useMemo(() => {
    const rows: { message: HeldMessage; outcome?: Decided['outcome'] }[] = (held ?? []).map((message) => ({
      message,
      outcome: decided[message.message_id]?.outcome,
    }));
    for (const d of Object.values(decided).sort((a, b) => a.index - b.index)) {
      if (rows.some((r) => r.message.message_id === d.message.message_id)) continue;
      rows.splice(Math.min(d.index, rows.length), 0, { message: d.message, outcome: d.outcome });
    }
    return rows;
  }, [held, decided]);

  const decide = async (message: HeldMessage, allow: boolean) => {
    setBusy(message.message_id);
    const index = (held ?? []).findIndex((m) => m.message_id === message.message_id);
    const failure = await useAutomodStore.getState().resolve(message.message_id, allow);
    setBusy(null);
    setHeldNote(failure);
    if (failure) return;
    const id = message.message_id;
    setDecided((cur) => ({ ...cur, [id]: { message, outcome: allow ? 'Allowed' : 'Denied', index: Math.max(0, index) } }));
    setTimeout(() => {
      setDecided((cur) => {
        const next = { ...cur };
        delete next[id];
        return next;
      });
    }, DECIDED_MS);
  };

  // Same as a held message: an answered request keeps its place a moment.
  const unbanRows = useMemo(() => {
    const rows: { request: UnbanRequest; outcome?: DecidedUnban['outcome'] }[] = unbanRequests.map((request) => ({
      request,
      outcome: unbanDecided[request.id]?.outcome,
    }));
    for (const d of Object.values(unbanDecided).sort((a, b) => a.index - b.index)) {
      if (rows.some((r) => r.request.id === d.request.id)) continue;
      rows.splice(Math.min(d.index, rows.length), 0, { request: d.request, outcome: d.outcome });
    }
    return rows;
  }, [unbanRequests, unbanDecided]);

  const answerUnban = async (request: UnbanRequest, approve: boolean) => {
    setBusy(request.id);
    const index = unbanRequests.findIndex((r) => r.id === request.id);
    const failure = await resolveUnban(request.id, approve, unbanReplies[request.id] ?? '');
    setBusy(null);
    setUnbanNote(failure);
    if (failure) return;
    const id = request.id;
    setUnbanDecided((cur) => ({ ...cur, [id]: { request, outcome: approve ? 'Approved' : 'Denied', index: Math.max(0, index) } }));
    setTimeout(() => {
      setUnbanDecided((cur) => {
        const next = { ...cur };
        delete next[id];
        return next;
      });
    }, DECIDED_MS);
  };

  const lift = async (entry: RoomModAction) => {
    setLiftPending(null);
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
      setBlocked(text);
      setPhrase('');
    } catch (err) {
      setPhraseNote(words(err));
    } finally {
      setBusy(null);
    }
  };

  /** Takes the phrase just blocked off the list again. */
  const unblock = async () => {
    const text = blocked;
    if (!text) return;
    setBusy('phrase');
    try {
      await invoke('remove_blocked_term', { broadcasterId: channelId, text });
      setPhraseNote(`Unblocked “${text}”`);
      setBlocked(null);
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
        <div className={heading}>{heldRows.length > 0 ? `Held by AutoMod · ${heldRows.length}` : 'Held by AutoMod'}</div>
        <div className="glass-panel divide-y divide-white/[0.06] overflow-hidden">
          {heldRows.length === 0 ? (
            <p className="px-3 py-3 text-[13px] text-textMuted">Nothing held right now.</p>
          ) : (
            heldRows.map(({ message: m, outcome }) => (
              <div key={m.message_id} className="px-3 py-2.5">
                <div className="flex items-center gap-1.5 text-[12px] font-semibold text-warning">
                  <Flag size={12} weight="fill" />
                  {m.label ?? 'Held by AutoMod'}
                </div>
                <p className="mt-1 text-[14px] leading-snug text-textPrimary [overflow-wrap:anywhere]">
                  <span className="font-bold">{m.user_name || m.user_login}</span>: {m.text}
                </p>
                {outcome ? (
                  <div className={`mt-2 flex h-[34px] items-center justify-center text-[14px] font-semibold ${outcome === 'Allowed' ? 'text-success' : 'text-error'}`}>
                    {outcome}
                  </div>
                ) : (
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
                    className="h-[34px] flex-1 rounded-[10px] bg-error/15 text-[14px] font-semibold text-[color-mix(in_srgb,var(--color-error)_70%,white)] disabled:opacity-60"
                  >
                    Deny
                  </button>
                </div>
                )}
              </div>
            ))
          )}
        </div>
        {heldNote && <p className={note}>{heldNote}</p>}

        <div className={heading}>{unbanRows.length > 0 ? `Unban requests · ${unbanRows.length}` : 'Unban requests'}</div>
        <div className="glass-panel divide-y divide-white/[0.06] overflow-hidden">
          {unbanRows.length === 0 ? (
            <p className="px-3 py-3 text-[13px] text-textMuted">No one is asking to be unbanned right now.</p>
          ) : (
            unbanRows.map(({ request: r, outcome }) => (
              <div key={r.id} className="px-3 py-2.5">
                <div className="flex items-baseline gap-2">
                  <span className="min-w-0 truncate text-[14px] font-bold text-textPrimary">{r.user_name || r.user_login}</span>
                  <span className="shrink-0 text-[12px] text-textMuted">{r.age}</span>
                </div>
                {r.text && (
                  <p className="mt-0.5 text-[14px] leading-snug text-textPrimary [overflow-wrap:anywhere]">{r.text}</p>
                )}
                {outcome ? (
                  <div className={`mt-2 flex h-[34px] items-center justify-center text-[14px] font-semibold ${outcome === 'Approved' ? 'text-success' : 'text-error'}`}>
                    {outcome}
                  </div>
                ) : (
                  <>
                    <input
                      value={unbanReplies[r.id] ?? ''}
                      onChange={(e) => setUnbanReplies((cur) => ({ ...cur, [r.id]: e.target.value }))}
                      maxLength={UNBAN_REPLY_MAX}
                      placeholder="Reply they'll see (optional)"
                      enterKeyHint="done"
                      aria-label={`Reply to ${r.user_name || r.user_login}`}
                      className="mt-2 h-9 w-full min-w-0 rounded-[10px] bg-white/[0.06] px-3 text-[14px] text-textPrimary outline-none placeholder:text-textMuted"
                    />
                    <div className="mt-2 flex gap-2">
                      <button
                        type="button"
                        disabled={busy != null}
                        onClick={() => void answerUnban(r, true)}
                        className="h-[34px] flex-1 rounded-[10px] bg-success/15 text-[14px] font-semibold text-success disabled:opacity-60"
                      >
                        Approve
                      </button>
                      <button
                        type="button"
                        disabled={busy != null}
                        onClick={() => void answerUnban(r, false)}
                        className="h-[34px] flex-1 rounded-[10px] bg-error/15 text-[14px] font-semibold text-[color-mix(in_srgb,var(--color-error)_70%,white)] disabled:opacity-60"
                      >
                        Deny
                      </button>
                    </div>
                  </>
                )}
              </div>
            ))
          )}
        </div>
        {unbanNote && <p className={note}>{unbanNote}</p>}

        <div className={`${heading} flex items-baseline justify-between`}>
          Recent actions
          {incoming.length > 0 && (
            <button
              type="button"
              onClick={showIncoming}
              className="flex items-center gap-1 text-[12px] font-semibold normal-case tracking-normal text-accent"
            >
              <ArrowUp size={11} weight="bold" />
              {incoming.length} new
            </button>
          )}
        </div>
        <div className="glass-panel divide-y divide-white/[0.06] overflow-hidden">
          {log == null ? null : log.length === 0 && incoming.length === 0 ? (
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
                {e.liftable && e.target_id && liftPending !== e.id && (
                  <button
                    type="button"
                    disabled={busy != null}
                    onClick={() => setLiftPending(e.id)}
                    className="shrink-0 px-1 py-1 text-[13px] font-semibold text-accent disabled:opacity-50"
                  >
                    {e.action === 'timeout' ? 'Lift timeout' : 'Unban'}
                  </button>
                )}
                {liftPending === e.id && (
                  // The tap turns the row's button into the question in place, so nothing shifts.
                  <div className="flex shrink-0 items-center gap-1">
                    <button
                      type="button"
                      onClick={() => setLiftPending(null)}
                      className="px-1 py-1 text-[13px] font-medium text-textSecondary"
                    >
                      Cancel
                    </button>
                    <button
                      type="button"
                      disabled={busy != null}
                      onClick={() => void lift(e)}
                      className="-mr-1 rounded-[8px] bg-accent/15 px-2 py-1 text-[13px] font-semibold text-accent disabled:opacity-50"
                    >
                      {e.action === 'timeout' ? 'Lift timeout' : 'Unban'}
                    </button>
                  </div>
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
        {phraseNote && (
          <div className="flex items-start gap-2 pr-3">
            <p className={`${note} flex-1`}>{phraseNote}</p>
            {blocked && (
              <button
                type="button"
                disabled={busy != null}
                onClick={() => void unblock()}
                className="shrink-0 px-1 pt-1.5 text-[12.5px] font-semibold text-accent disabled:opacity-50"
              >
                Undo
              </button>
            )}
          </div>
        )}
      </div>
    </MobileSheet>
  );
};
