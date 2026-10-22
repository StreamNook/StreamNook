// The desktop user card's Twitch moderator section. Rust decides every button
// and word (services/chatter_card.rs): the four timeout lengths, Ban or Unban
// in one slot, Warn when the sign-in may warn, the viewer's own buttons, the
// low-trust line with its treatments, and the history. The card asks twice:
// first without the history so the buttons draw at once, then with it, which
// swaps Ban for Unban or Lift timeout in place and fills the history below.
import { useCallback, useEffect, useRef, useState, type MouseEvent, type ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import { AlertTriangle, Ban, Check, Clock, MessageSquare, RotateCcw, Trash2, type LucideIcon } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import { Tooltip } from '../ui/Tooltip';
import { ACCENT_BUTTON, ACCENT_FILL, DANGER_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import { armedFor, moderateWithToast } from '../../services/moderation';
import {
  commandFor,
  HISTORY_PREVIEW,
  loadCardTools,
  REASON_MAX,
  requestFor,
  setLowTrust,
  shownHistory,
  type CardButton,
  type CardTools,
  type HistoryEntry,
  type LowTrustAction,
} from '../../utils/cardActions';

const KIND_MARK: Record<HistoryEntry['kind'], { Icon: LucideIcon; tone: string }> = {
  ban: { Icon: Ban, tone: 'text-error bg-error/15' },
  timeout: { Icon: Clock, tone: 'text-warning bg-warning/15' },
  unban: { Icon: RotateCcw, tone: 'text-textSecondary bg-glass/40' },
  untimeout: { Icon: RotateCcw, tone: 'text-textSecondary bg-glass/40' },
  warn: { Icon: AlertTriangle, tone: 'text-info bg-info/15' },
  acknowledged: { Icon: Check, tone: 'text-info bg-info/15' },
  deleted: { Icon: Trash2, tone: 'text-textSecondary bg-glass/40' },
  comment: { Icon: MessageSquare, tone: 'text-info bg-info/15' },
};

/** The card buttons the viewer saved, before Rust answers: their rows are
 *  held open so nothing under them moves when the buttons arrive. */
const MAX_CUSTOM = 6;

// The glaze's buttons: capsules lit by glaze-inset. Ban is the one filled
// button; everything else is the quieter pill.
const buttonShape = 'h-8 min-w-0 w-full px-2 flex items-center justify-center';
const buttonBase = `${GLAZE_PILL} ${buttonShape} text-xs`;
const dangerBase = `${ACCENT_BUTTON} ${buttonShape} !text-xs`;
// Low trust and History each start under a hairline with room above, so the
// three parts of the section read apart.
const sectionClass = 'mt-4 pt-3.5 border-t border-borderSubtle';
const sectionTitleClass = 'text-[11px] uppercase font-bold text-textSecondary tracking-wider';

export function TwitchCardModerate({
  userId,
  login,
  displayName,
  channel,
  broadcasterId,
  blockedNote,
  roleControls,
  onPreFillCommand,
  onClose,
}: {
  userId: string;
  login: string;
  displayName: string;
  /** The channel's login. */
  channel: string;
  broadcasterId: string;
  /** Why nothing can be done to this chatter, when Twitch would refuse. */
  blockedNote: string | null;
  /** The broadcaster's Mod and VIP toggles. */
  roleControls: ReactNode;
  onPreFillCommand?: (cmd: string) => void;
  onClose: () => void;
}) {
  const [tools, setTools] = useState<CardTools | null>(null);
  const [answered, setAnswered] = useState(false);
  const [running, setRunning] = useState<string | null>(null);
  const [asking, setAsking] = useState<CardButton | null>(null);
  // A Ban waiting for its second press (Ban confirmation on), with Rust's words.
  const [armed, setArmed] = useState<{ id: string; prompt: string } | null>(null);
  const [reason, setReason] = useState('');
  const [historyOpen, setHistoryOpen] = useState(false);
  const savedReasons = useAppStore((s) => s.settings.moderation?.saved_ban_reasons);
  const savedButtons = useAppStore((s) => s.settings.moderation?.card_actions?.length ?? 0);
  const request = useRef(0);
  const name = displayName || login;

  // Each load supersedes the last; the quick answer never replaces the full
  // one when they land out of order.
  const load = useCallback(
    (quickFirst: boolean) => {
      if (blockedNote) return;
      const id = ++request.current;
      let fullIn = false;
      // The card mounts only for a viewer the shell already knows moderates.
      const args = { channel, channelId: broadcasterId, userId, viewerModerates: true };
      if (quickFirst) {
        loadCardTools({ ...args, history: false })
          .then((t) => {
            if (request.current !== id || fullIn) return;
            setTools(t);
            setAnswered(true);
          })
          .catch((err) => Logger.debug('[CardModerate] quick tools failed:', err));
      }
      loadCardTools({ ...args, history: true })
        .then((t) => {
          if (request.current !== id) return;
          fullIn = true;
          setTools(t);
        })
        .catch((err) => Logger.debug('[CardModerate] tools failed:', err))
        .finally(() => request.current === id && setAnswered(true));
    },
    [blockedNote, channel, broadcasterId, userId],
  );

  // The card keys this section by chatter and channel, so a new chatter
  // starts from empty state; this only asks.
  useEffect(() => {
    load(true);
  }, [load]);

  // A treatment changed by any moderator (this card included) reaches the
  // card through EventSub; only the low-trust part is asked again.
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<{ channel?: string | null; user_id?: string | null }>('eventsub://suspicious-user-update', (e) => {
      if (e.payload.user_id !== userId || (e.payload.channel ?? '').toLowerCase() !== channel.toLowerCase()) return;
      void loadCardTools({ channel, channelId: broadcasterId, userId, history: false, viewerModerates: true })
        .then((t) => setTools((prev) => (prev ? { ...prev, low_trust: t.low_trust } : prev)))
        .catch(() => {});
    }).then((un) => (gone ? un() : (off = un)));
    return () => {
      gone = true;
      off?.();
    };
  }, [channel, broadcasterId, userId]);


  const run = async (button: CardButton, typed?: string) => {
    if (running) return;
    setRunning(button.id);
    try {
      const done = await moderateWithToast(requestFor(button, { broadcasterId, userId, name: login }, typed));
      if (done?.confirmMs) {
        // Nothing happened yet: the button says so until Rust's window closes.
        const prompt = done.message || armed?.prompt || 'Press again to ban';
        setArmed({ id: button.id, prompt });
        window.setTimeout(() => setArmed((a) => (a?.id === button.id ? null : a)), armedFor(done));
        return;
      }
      setArmed(null);
      // Ban or Unban and the history follow what is now in force.
      if (done) load(false);
    } finally {
      setRunning(null);
    }
  };

  const press = (button: CardButton, e: MouseEvent) => {
    if (e.shiftKey && onPreFillCommand) {
      // A timeout or ban fills in the first saved reason when it has none.
      const fill = button.kind === 'timeout' || button.kind === 'ban' ? (button.reason ?? savedReasons?.[0] ?? null) : null;
      onPreFillCommand(commandFor(button, login, fill));
      onClose();
      return;
    }
    // A press acts at once; only a warning, which Twitch refuses without a
    // reason, asks for one first.
    if (button.asks_reason) {
      setReason('');
      setAsking(button);
      return;
    }
    void run(button);
  };

  const sendReason = () => {
    const b = asking;
    if (!b || !reason.trim()) return;
    setAsking(null);
    void run(b, reason);
  };

  const treat = async (action: LowTrustAction) => {
    if (running) return;
    setRunning(action.id);
    try {
      await setLowTrust(action, broadcasterId, userId, channel);
      useAppStore.getState().addToast(`${name}: ${action.done}`, 'success');
    } catch (err) {
      useAppStore.getState().addToast(typeof err === 'string' ? err : 'Twitch did not take that change', 'error');
    } finally {
      setRunning(null);
    }
  };

  const moderate = tools?.moderate ?? null;
  // Rust said there is nothing to moderate here (its own check of who may);
  // the broadcaster's Mod and VIP switches still apply.
  if (!blockedNote && answered && !moderate) return <>{roleControls}</>;

  const buttonEl = (b: CardButton) => {
    const isArmed = armed?.id === b.id;
    return (
      <Tooltip
        key={b.id}
        content={`${b.spoken}. Shift-click to edit it in chat first`}
        side="top"
        disabled={isArmed}
      >
        <button
          type="button"
          disabled={running != null}
          onClick={(e) => press(b, e)}
          aria-label={isArmed ? (armed?.prompt ?? b.spoken) : b.spoken}
          className={`${b.danger ? dangerBase : buttonBase}${isArmed ? ' brightness-150' : ''}`}
          style={b.danger ? DANGER_FILL : undefined}
        >
          <span className="truncate">{running === b.id ? '…' : isArmed ? 'Press again to ban' : b.label}</span>
        </button>
      </Tooltip>
    );
  };

  const placeholder = (key: string) => (
    <div key={key} className={`${buttonShape} rounded-full bg-white/[0.05]`} aria-hidden />
  );

  const lengths = moderate?.buttons.filter((b) => b.kind === 'timeout') ?? [];
  const second = moderate?.buttons.filter((b) => b.kind !== 'timeout') ?? [];
  const customRows = moderate
    ? Math.ceil(moderate.custom.length / 3)
    : Math.ceil(Math.min(savedButtons, MAX_CUSTOM) / 3);
  const history = moderate?.history ?? null;
  const lowTrust = tools?.low_trust ?? null;
  const entries = shownHistory(history, historyOpen);

  return (
    <div className="pt-3 border-t border-error/20">
      <div className="flex items-center gap-1.5 mb-3">
        <AlertTriangle className="w-3.5 h-3.5 text-error" />
        <span className="text-[11px] uppercase font-bold text-error tracking-wider">Moderator Actions</span>
      </div>

      {blockedNote ? (
        <p className="mb-3 text-xs leading-relaxed text-textSecondary">{blockedNote}</p>
      ) : (
        <div className="space-y-2">
          <div className="grid grid-cols-4 gap-2">
            {moderate ? lengths.map(buttonEl) : [0, 1, 2, 3].map((i) => placeholder(`l${i}`))}
          </div>
          <div className={`grid gap-2 ${!moderate || second.length > 1 ? 'grid-cols-2' : 'grid-cols-1'}`}>
            {moderate ? second.map(buttonEl) : [placeholder('ban'), placeholder('warn')]}
          </div>
          {Array.from({ length: customRows }, (_, row) => (
            <div key={`c${row}`} className="grid grid-cols-3 gap-2">
              {moderate
                ? moderate.custom.slice(row * 3, row * 3 + 3).map(buttonEl)
                : [0, 1, 2].map((i) => placeholder(`c${row}-${i}`))}
            </div>
          ))}

          {asking && (
            // Its own tile under the buttons, so it reads as one step: say
            // why, then send.
            <div className="mt-1 space-y-2.5 rounded-2xl bg-white/[0.04] p-3">
              <label htmlFor="card-reason" className="block text-xs font-semibold text-textPrimary">
                {asking.label}: the reason {name} will see
              </label>
              <div className="flex items-center gap-2">
                <input
                  id="card-reason"
                  autoFocus
                  value={reason}
                  maxLength={REASON_MAX}
                  onChange={(e) => setReason(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      sendReason();
                    } else if (e.key === 'Escape') {
                      e.stopPropagation();
                      setAsking(null);
                    }
                  }}
                  placeholder="Type a reason or pick one below"
                  className="glass-input h-8 min-w-0 flex-1 rounded-full px-3.5 text-xs text-textPrimary placeholder-textSecondary"
                />
                <button
                  type="button"
                  disabled={!reason.trim() || running != null}
                  onClick={sendReason}
                  className={`${ACCENT_BUTTON} h-8 shrink-0 px-4 !text-xs`}
                  style={ACCENT_FILL}
                >
                  {asking.label}
                </button>
                <button
                  type="button"
                  onClick={() => setAsking(null)}
                  className="h-8 shrink-0 px-2 text-xs text-textSecondary hover:text-textPrimary"
                >
                  Cancel
                </button>
              </div>
              {!!savedReasons?.length && (
                <div className="flex flex-wrap gap-2">
                  {savedReasons.map((r) => (
                    <button
                      key={r}
                      type="button"
                      onClick={() => setReason(r.slice(0, REASON_MAX))}
                      className={`${GLAZE_PILL} max-w-full truncate px-3 py-1.5 text-[12px] !font-medium`}
                    >
                      {r}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {roleControls}

      {!blockedNote && (
        <div className={sectionClass}>
          <div className="flex items-baseline justify-between gap-2 mb-2">
            <span className={sectionTitleClass}>Low trust</span>
          </div>
          <p
            className={`mb-2.5 text-xs leading-relaxed ${
              lowTrust?.status === 'restricted'
                ? 'font-medium text-error'
                : lowTrust?.status === 'monitored'
                  ? 'font-medium text-warning'
                  : 'text-textSecondary'
            }`}
          >
            {/* Nothing is claimed until Rust has answered; the line keeps its
                height so the buttons under it stay put. */}
            {tools ? (lowTrust?.line ?? 'Twitch has not flagged them while you were here.') : ' '}
          </p>
          <div className="grid grid-cols-2 gap-2">
            {lowTrust
              ? lowTrust.actions.map((a) => (
                  <Tooltip key={a.id} content={a.spoken} side="top">
                    <button
                      type="button"
                      disabled={running != null}
                      onClick={() => void treat(a)}
                      aria-label={a.spoken}
                      className={buttonBase}
                    >
                      {running === a.id ? '…' : a.label}
                    </button>
                  </Tooltip>
                ))
              : [0, 1].map((i) => placeholder(`lt${i}`))}
          </div>
        </div>
      )}

      {!blockedNote && (
        <div className={sectionClass}>
          <div className="flex items-baseline justify-between gap-2 mb-2">
            <span className={sectionTitleClass}>History</span>
            {history?.status === 'available' && history.entries.length > 0 && (
              <span className="text-[12px] text-textSecondary tabular-nums">{history.entries.length}</span>
            )}
          </div>
          {history?.current && (
            <p className="mb-2.5 flex items-center gap-1.5 text-xs font-semibold text-error">
              {history.current.permanent ? <Ban size={13} /> : <Clock size={13} />}
              {history.current.label}
            </p>
          )}
          {!history || history.status === 'not_loaded' ? (
            <p className="text-xs text-textSecondary">Asking Twitch…</p>
          ) : history.status !== 'available' ? (
            <p className="text-xs leading-relaxed text-textSecondary">
              {history.note ?? 'Twitch did not answer. Try again in a moment.'}
            </p>
          ) : history.entries.length === 0 ? (
            <p className="text-xs text-textSecondary">Nothing on record here.</p>
          ) : (
            <div className="divide-y divide-white/[0.06] overflow-hidden rounded-2xl bg-white/[0.04]">
              {entries.map((e) => {
                const { Icon, tone } = KIND_MARK[e.kind] ?? KIND_MARK.comment;
                const meta = [e.by ? `by ${e.by}` : null, e.when || null].filter(Boolean).join(' · ');
                return (
                  <div key={e.id || `${e.kind}-${e.at_ms}`} className="flex items-start gap-2.5 px-3 py-2.5">
                    <span className={`mt-px flex h-6 w-6 shrink-0 items-center justify-center rounded-md ${tone}`}>
                      <Icon size={12} />
                    </span>
                    <div className="min-w-0 flex-1 space-y-0.5">
                      <div className="text-xs font-semibold text-textPrimary">{e.title}</div>
                      {meta && <div className="text-[12px] text-textSecondary">{meta}</div>}
                      {e.quote && (
                        <div className="pt-0.5 text-xs leading-relaxed text-textSecondary [overflow-wrap:anywhere]">
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
                  className="block w-full px-3 py-2 text-left text-xs font-medium text-accent hover:underline"
                >
                  {historyOpen ? 'Show fewer' : `Show all ${history.entries.length}`}
                </button>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
