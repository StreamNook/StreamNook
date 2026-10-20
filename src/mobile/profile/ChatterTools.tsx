// The user card's own sections, under the person's sub standing: Moderate (the
// four timeout lengths and Ban or Unban, then the viewer's own buttons) and
// History in a channel the viewer moderates; then the note and name color only
// the viewer sees. Rust decides all of it (services/chatter_card.rs): who may
// be moderated, which buttons, what each runs, and the history's words. Every
// button goes through the one mod door, `moderate_chatter`.
import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowCounterClockwise, Check, ChatText, Clock, Prohibit, Trash, Warning } from 'phosphor-react';
import { Logger } from '../../utils/logger';
import { SectionHeader } from './IdentityParts';

interface CardButton {
  id: string;
  label: string;
  kind: 'timeout' | 'ban' | 'unban' | 'warn';
  /** What `moderate_chatter` takes as `action`, passed through untouched. */
  action: Record<string, unknown>;
  reason: string | null;
  asks_reason: boolean;
  danger: boolean;
  spoken: string;
}

interface HistoryEntry {
  id: string;
  kind: 'ban' | 'timeout' | 'unban' | 'untimeout' | 'warn' | 'acknowledged' | 'deleted' | 'comment';
  title: string;
  by: string | null;
  at_ms: number;
  when: string;
  quote: string | null;
}

interface ModHistory {
  status: 'available' | 'no_access' | 'signed_out' | 'failed';
  entries: HistoryEntry[];
  current: { permanent: boolean; label: string } | null;
  note: string | null;
}

interface CardTools {
  note: string | null;
  nickname: string | null;
  color: string | null;
  swatches: string[];
  moderate: { buttons: CardButton[]; custom: CardButton[]; history: ModHistory } | null;
}

interface ModOutcome {
  message: string;
  undoable: boolean;
}

const HISTORY_PREVIEW = 3;

const KIND_MARK: Record<HistoryEntry['kind'], { Icon: typeof Clock; tone: string }> = {
  ban: { Icon: Prohibit, tone: 'text-error bg-error/15' },
  timeout: { Icon: Clock, tone: 'text-warning bg-warning/15' },
  unban: { Icon: ArrowCounterClockwise, tone: 'text-textMuted bg-surface' },
  untimeout: { Icon: ArrowCounterClockwise, tone: 'text-textMuted bg-surface' },
  warn: { Icon: Warning, tone: 'text-info bg-info/15' },
  acknowledged: { Icon: Check, tone: 'text-info bg-info/15' },
  deleted: { Icon: Trash, tone: 'text-textMuted bg-surface' },
  comment: { Icon: ChatText, tone: 'text-info bg-info/15' },
};

export function ChatterTools({
  userId,
  login,
  displayName,
  channelId,
  channelName,
}: {
  userId: string;
  login: string;
  displayName: string;
  channelId?: string | null;
  channelName?: string | null;
}) {
  const [tools, setTools] = useState<CardTools | null>(null);
  const [running, setRunning] = useState<string | null>(null);
  const [result, setResult] = useState<{ text: string; undo: CardButton | null } | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [note, setNote] = useState('');
  const savedNote = useRef('');
  const [armedBan, setArmedBan] = useState<string | null>(null);
  const [warnFor, setWarnFor] = useState<CardButton | null>(null);
  const [warnReason, setWarnReason] = useState('');

  const load = useCallback(async () => {
    if (!userId) return;
    try {
      const answer = await invoke<CardTools>('get_chatter_card_tools', {
        provider: 'twitch',
        channel: channelName ?? null,
        channelId: channelId ?? null,
        userId,
      });
      setTools(answer);
      savedNote.current = answer.note ?? '';
      setNote(answer.note ?? '');
    } catch (err) {
      Logger.debug('[ChatterTools] card tools unavailable:', err);
    }
  }, [userId, channelId, channelName]);

  useEffect(() => {
    void load();
  }, [load]);

  // A Ban tap arms it; a second tap within 3 s bans. The armed state is the
  // visible "are you sure", and waiting it out is the cancel.
  useEffect(() => {
    if (!armedBan) return;
    const t = window.setTimeout(() => setArmedBan(null), 3000);
    return () => window.clearTimeout(t);
  }, [armedBan]);

  const run = async (button: CardButton, reason?: string) => {
    if (running || !channelId) return;
    setRunning(button.id);
    try {
      const outcome = await invoke<ModOutcome>('moderate_chatter', {
        request: {
          provider: 'twitch',
          broadcasterId: channelId,
          targetUserId: userId,
          targetName: displayName,
          action: button.action,
          ...((reason ?? button.reason)?.trim() ? { reason: (reason ?? button.reason)!.trim() } : {}),
        },
      });
      const undo: CardButton | null = outcome.undoable
        ? {
            id: 'undo',
            label: 'Undo',
            kind: 'unban',
            action: { kind: 'unban', timeout: button.kind === 'timeout' },
            reason: null,
            asks_reason: false,
            danger: false,
            spoken: 'Undo',
          }
        : null;
      setResult({ text: outcome.message, undo });
      // The history and Ban or Unban follow what is now in force.
      await load();
    } catch (err) {
      setResult({ text: typeof err === 'string' ? err : 'That did not go through', undo: null });
    } finally {
      setRunning(null);
    }
  };

  const tap = (button: CardButton) => {
    if (button.danger && armedBan !== button.id) {
      setArmedBan(button.id);
      return;
    }
    setArmedBan(null);
    if (button.asks_reason) {
      setWarnReason('');
      setWarnFor(button);
      return;
    }
    void run(button);
  };

  const saveNote = () => {
    const next = note.trim();
    if (next === savedNote.current) return;
    savedNote.current = next;
    void invoke('set_user_note', { userId, note: next }).catch((err) => Logger.warn('[ChatterTools] note not saved:', err));
  };

  const setColor = (color: string | null) => {
    setTools((t) => (t ? { ...t, color } : t));
    void invoke('set_chat_user_override', { userId, username: login, field: 'color', value: color }).catch((err) =>
      Logger.warn('[ChatterTools] color not saved:', err),
    );
  };

  if (!tools) return null;
  const moderate = tools.moderate;

  const buttonRow = (buttons: CardButton[], fill?: number) => (
    <div className="flex gap-[7px]">
      {buttons.map((b) => {
        const armed = armedBan === b.id;
        return (
          <button
            key={b.id}
            type="button"
            disabled={running != null}
            onClick={() => tap(b)}
            aria-label={armed ? `Confirm: ${b.spoken}` : b.spoken}
            className={`flex h-[38px] min-w-0 flex-1 items-center justify-center truncate rounded-xl border px-1 text-[14px] font-semibold transition-colors active:scale-[0.97] disabled:opacity-60 ${
              b.danger
                ? armed
                  ? 'border-error bg-error text-background'
                  : 'border-error/25 bg-error/[0.12] text-error'
                : 'border-white/[0.06] bg-surface text-textPrimary'
            }`}
          >
            {running === b.id ? '…' : armed ? 'Tap to ban' : b.label}
          </button>
        );
      })}
      {fill != null &&
        Array.from({ length: Math.max(0, fill - buttons.length) }, (_, i) => <span key={`pad-${i}`} className="flex-1" />)}
    </div>
  );

  const history = moderate?.history;
  const shownHistory = history ? (historyOpen ? history.entries : history.entries.slice(0, HISTORY_PREVIEW)) : [];

  return (
    <>
      {moderate && (
        <div className="mt-5">
          <SectionHeader title="Moderate" className="mb-2" />
          <div className="flex flex-col gap-[7px]">
            {buttonRow(moderate.buttons)}
            {Array.from({ length: Math.ceil(moderate.custom.length / 3) }, (_, row) =>
              <div key={`custom-${row}`}>{buttonRow(moderate.custom.slice(row * 3, row * 3 + 3), 3)}</div>,
            )}
          </div>
          {warnFor && (
            <div className="mt-2 flex gap-2">
              <input
                autoFocus
                value={warnReason}
                onChange={(e) => setWarnReason(e.target.value)}
                placeholder="Reason they will see"
                className="min-w-0 flex-1 rounded-xl bg-surface px-3 text-[14px] text-textPrimary outline-none placeholder:text-textMuted"
              />
              <button
                type="button"
                disabled={!warnReason.trim() || running != null}
                onClick={() => {
                  const b = warnFor;
                  setWarnFor(null);
                  void run(b, warnReason);
                }}
                className="h-[38px] shrink-0 rounded-xl bg-info px-3 text-[14px] font-semibold text-background disabled:opacity-50"
              >
                Warn
              </button>
              <button type="button" onClick={() => setWarnFor(null)} className="h-[38px] shrink-0 px-2 text-[14px] text-textSecondary">
                Cancel
              </button>
            </div>
          )}
          {result && (
            <div className="mt-2 flex items-center gap-2 text-[12.5px]" role="status">
              <span className="min-w-0 flex-1 line-clamp-2 text-textSecondary">{result.text}</span>
              {result.undo && (
                <button
                  type="button"
                  disabled={running != null}
                  onClick={() => result.undo && void run(result.undo)}
                  className="shrink-0 px-1 font-semibold text-accent disabled:opacity-50"
                >
                  Undo
                </button>
              )}
            </div>
          )}
        </div>
      )}

      {history && (
        <div className="mt-5">
          <SectionHeader title="History" detail={history.entries.length ? String(history.entries.length) : null} className="mb-2" />
          {history.current && (
            <p className="mb-2 flex items-center gap-1.5 text-[13px] font-semibold text-error">
              {history.current.permanent ? <Prohibit size={14} weight="bold" /> : <Clock size={14} weight="bold" />}
              {history.current.label}
            </p>
          )}
          {history.status !== 'available' ? (
            <p className="text-[12.5px] text-textMuted">{history.note ?? 'Twitch did not answer. Try again in a moment.'}</p>
          ) : history.entries.length === 0 ? (
            <p className="text-[12.5px] text-textMuted">Nothing on record here.</p>
          ) : (
            <div className="glass-panel divide-y divide-white/[0.06] overflow-hidden">
              {shownHistory.map((e) => {
                const { Icon, tone } = KIND_MARK[e.kind] ?? KIND_MARK.comment;
                const meta = [e.by ? `by ${e.by}` : null, e.when || null].filter(Boolean).join(' · ');
                return (
                  <div key={e.id || `${e.kind}-${e.at_ms}`} className="flex items-start gap-2.5 px-3 py-2.5">
                    <span className={`flex h-[26px] w-[26px] shrink-0 items-center justify-center rounded-lg ${tone}`}>
                      <Icon size={13} weight="bold" />
                    </span>
                    <div className="min-w-0 flex-1">
                      <div className="text-[14px] font-semibold text-textPrimary">{e.title}</div>
                      {meta && <div className="text-[12.5px] text-textMuted">{meta}</div>}
                      {e.quote && (
                        <div className="mt-0.5 text-[13px] leading-snug text-textSecondary [overflow-wrap:anywhere]">
                          “{e.quote}”
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
              {history.entries.length > HISTORY_PREVIEW && (
                <button
                  type="button"
                  onClick={() => setHistoryOpen((o) => !o)}
                  className="block h-10 w-full px-3 text-left text-[13px] font-medium text-accent"
                >
                  {historyOpen ? 'Show fewer' : `Show all ${history.entries.length}`}
                </button>
              )}
            </div>
          )}
        </div>
      )}

      <div className="mt-5">
        <SectionHeader title="Only you see this" className="mb-2" />
        <div className="glass-panel overflow-hidden">
          <textarea
            value={note}
            onChange={(e) => setNote(e.target.value)}
            onBlur={saveNote}
            rows={1}
            placeholder="Add a note"
            className="block max-h-[110px] w-full resize-none bg-transparent px-3 py-[11px] text-[14px] text-textPrimary outline-none [field-sizing:content] placeholder:text-textMuted"
          />
          <div className="flex min-h-[46px] items-center gap-2 border-t border-white/[0.06] px-3">
            <span className="shrink-0 text-[14px] text-textPrimary">Name color</span>
            <div className="flex min-w-0 flex-1 justify-end gap-[7px] overflow-x-auto py-1 [scrollbar-width:none]">
              {tools.swatches.map((hex) => {
                const on = tools.color?.toLowerCase() === hex.toLowerCase();
                return (
                  <button
                    key={hex}
                    type="button"
                    onClick={() => setColor(on ? null : hex)}
                    aria-label={`Color ${hex}`}
                    aria-pressed={on}
                    className={`h-[26px] w-[26px] shrink-0 rounded-full border-2 p-[3px] ${on ? 'border-textPrimary' : 'border-transparent'}`}
                  >
                    <span className="block h-full w-full rounded-full" style={{ background: hex }} />
                  </button>
                );
              })}
            </div>
            {tools.color && (
              <button type="button" onClick={() => setColor(null)} className="shrink-0 text-[13px] font-medium text-textSecondary">
                Reset
              </button>
            )}
          </div>
        </div>
      </div>
    </>
  );
}
