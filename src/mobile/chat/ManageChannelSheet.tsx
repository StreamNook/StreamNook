// The phone's stream manager: one sheet behind the chat menu's "Manage
// channel" row, shown to the broadcaster and the channel's moderators. A page
// of sections (Stream, Chat, Moderation), each opening its form in the same sheet. Rust
// builds the view and runs every write (services/stream_manager.rs); the forms
// are the desktop builders' own (ManagerForm), so both apps say the same thing.
import React, { useState } from 'react';
import { CaretLeft, CaretRight } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { useAppStore } from '../../stores/AppStore';
import { ManagerForm, TITLES, type ManagerPanelKind } from '../../components/chat/StreamManagerPanel';
import PollPredictionComposer from '../../components/chat/PollPredictionComposer';
import { managerAct, useStreamManager, type ManagerAction, type ManagerSection } from '../../services/streamManager';
import { BlockedTermsPage } from './BlockedTermsPage';

type Page = 'home' | 'poll' | 'prediction' | 'blocked' | ManagerPanelKind;

interface Props {
  open: boolean;
  onClose: () => void;
  channelId: string;
  channelLogin: string;
}

export const ManageChannelSheet: React.FC<Props> = ({ open, onClose, channelId, channelLogin }) => {
  const addToast = useAppStore((s) => s.addToast);
  const { view, queue, setQueue, error, apply } = useStreamManager(channelId, channelLogin, open);
  const [page, setPage] = useState<Page>('home');
  const [busy, setBusy] = useState(false);
  const [confirmShield, setConfirmShield] = useState(false);

  const close = () => {
    setPage('home');
    setConfirmShield(false);
    onClose();
  };

  const run = async (action: ManagerAction) => {
    if (busy) return;
    setBusy(true);
    try {
      const outcome = await managerAct(channelId, channelLogin, action);
      apply(outcome);
      addToast(outcome.message, outcome.ok ? 'success' : 'error');
    } catch (e) {
      addToast(typeof e === 'string' ? e : 'That did not go through', 'error');
    } finally {
      setBusy(false);
    }
  };

  const title =
    page === 'home'
      ? 'Manage channel'
      : page === 'poll'
        ? 'Start a poll'
        : page === 'prediction'
          ? 'Start a prediction'
          : page === 'blocked'
            ? 'Blocked terms'
            : TITLES[page].title;

  // Blocked terms are a moderator's tool; editors alone cannot read them.
  const moderates = view?.role === 'broadcaster' || view?.role === 'moderator' || view?.role === 'moderator_editor';

  const row = 'sn-touch w-full flex items-center gap-3 px-3 min-h-[48px] text-left active:opacity-70 disabled:opacity-45';
  const group = 'rounded-2xl bg-white/[0.05] overflow-hidden divide-y divide-borderSubtle';
  const groupLabel = 'px-3 pb-1.5 text-[12px] font-semibold uppercase tracking-wide text-textMuted';

  const openSection = (id: ManagerSection) => {
    if (id === 'poll' && view?.poll) return;
    if (id === 'prediction' && view?.prediction) return;
    setPage(id === 'chat' ? 'home' : (id as Page));
  };

  return (
    <MobileSheet open={open} onClose={close} maxHeightFraction={0.92} fill>
      <div className="flex flex-col h-full min-h-0">
        <div className="relative flex items-center justify-center h-11 shrink-0 px-2">
          {page !== 'home' && (
            <button
              onClick={() => setPage('home')}
              className="sn-touch absolute left-1 flex items-center gap-0.5 px-2 text-[15px] text-accent active:opacity-70"
            >
              <CaretLeft size={18} weight="bold" />
              Back
            </button>
          )}
          <span className="text-[16px] font-semibold text-textPrimary">{title}</span>
        </div>

        <div className="flex-1 min-h-0 overflow-y-auto px-3 pb-4 sn-manage-touch">
          {!view && (
            <p className="px-1 py-6 text-center text-[14px] text-textMuted">{error ?? 'Loading…'}</p>
          )}

          {view && page === 'home' && (
            <div className="flex flex-col gap-4 pt-1">
              <p className="px-1 text-[13px] text-textMuted">{view.role_line}</p>

              {view.sections.filter((s) => s.id !== 'announce').length > 0 && (
                <div>
                  <div className={groupLabel}>Stream</div>
                  <div className={group}>
                    {view.sections
                      .filter((s) => s.id !== 'announce')
                      .map((s) => {
                        const running =
                          (s.id === 'poll' && view.poll) || (s.id === 'prediction' && view.prediction);
                        return (
                          <div key={s.id}>
                            <button onClick={() => openSection(s.id)} className={row}>
                              <span className="flex-1 min-w-0">
                                <span className="block text-[15px] text-textPrimary truncate">{s.label}</span>
                                {s.detail && <span className="block text-[12.5px] text-textMuted truncate">{s.detail}</span>}
                              </span>
                              {s.badge && <span className="text-[13px] tabular-nums text-textMuted">{s.badge}</span>}
                              {!running && <CaretRight size={16} className="text-textMuted" />}
                            </button>
                            {s.id === 'poll' && view.poll && (
                              <div className="flex gap-2 px-3 pb-3">
                                <PillButton disabled={busy} onClick={() => run({ kind: 'end_poll', show_results: true })}>
                                  End
                                </PillButton>
                                <PillButton danger disabled={busy} onClick={() => run({ kind: 'end_poll', show_results: false })}>
                                  Remove
                                </PillButton>
                              </div>
                            )}
                            {s.id === 'prediction' && view.prediction && (
                              <div className="flex flex-wrap gap-2 px-3 pb-3">
                                {view.prediction.status === 'active' && (
                                  <PillButton disabled={busy} onClick={() => run({ kind: 'lock_prediction' })}>
                                    Lock
                                  </PillButton>
                                )}
                                {view.prediction.status === 'locked' &&
                                  view.prediction.outcomes.map((o) => (
                                    <PillButton
                                      key={o.id}
                                      disabled={busy}
                                      onClick={() => run({ kind: 'resolve_prediction', outcome_id: o.id })}
                                    >
                                      {o.title} won
                                    </PillButton>
                                  ))}
                                <PillButton danger disabled={busy} onClick={() => run({ kind: 'cancel_prediction' })}>
                                  Refund
                                </PillButton>
                              </div>
                            )}
                          </div>
                        );
                      })}
                  </div>
                </div>
              )}

              {view.chat && (
                <div>
                  <div className={groupLabel}>Chat</div>
                  <div className={group}>
                    {view.chat.modes.map((m) => (
                      <div key={m.kind} className="flex items-center gap-3 px-3 min-h-[48px]">
                        <span className="flex-1 text-[15px] text-textPrimary">{m.label}</span>
                        {m.on && m.options.length > 0 && (
                          <select
                            value={m.value ?? ''}
                            disabled={busy}
                            onChange={(e) => run({ kind: 'chat_mode', mode: m.kind, on: true, value: Number(e.target.value) })}
                            className="bg-transparent text-[13.5px] text-textMuted text-right outline-none"
                          >
                            {m.options.map((o) => (
                              <option key={o.value} value={o.value}>
                                {o.label}
                              </option>
                            ))}
                          </select>
                        )}
                        <Switch
                          on={m.on}
                          disabled={busy}
                          label={m.label}
                          onToggle={() => run({ kind: 'chat_mode', mode: m.kind, on: !m.on, value: m.value })}
                        />
                      </div>
                    ))}
                    {view.chat.shield !== null && (
                      <div className="px-3 min-h-[48px] flex items-center gap-3">
                        <span className="flex-1 text-[15px] text-textPrimary">
                          {confirmShield ? 'Turn on Shield Mode? It can clear chat.' : 'Shield Mode'}
                        </span>
                        {confirmShield ? (
                          <>
                            <PillButton
                              danger
                              disabled={busy}
                              onClick={() => {
                                setConfirmShield(false);
                                void run({ kind: 'shield', on: true });
                              }}
                            >
                              Turn on
                            </PillButton>
                            <PillButton disabled={busy} onClick={() => setConfirmShield(false)}>
                              No
                            </PillButton>
                          </>
                        ) : (
                          <Switch
                            on={view.chat.shield}
                            disabled={busy}
                            label="Shield Mode"
                            onToggle={() =>
                              view.chat?.shield ? void run({ kind: 'shield', on: false }) : setConfirmShield(true)
                            }
                          />
                        )}
                      </div>
                    )}
                    {view.sections.some((s) => s.id === 'announce') && (
                      <button onClick={() => setPage('announce')} className={row}>
                        <span className="flex-1 text-[15px] text-textPrimary">Announce</span>
                        <CaretRight size={16} className="text-textMuted" />
                      </button>
                    )}
                  </div>
                </div>
              )}

              {moderates && (
                <div>
                  <div className={groupLabel}>Moderation</div>
                  <div className={group}>
                    <button onClick={() => setPage('blocked')} className={row}>
                      <span className="flex-1 text-[15px] text-textPrimary">Blocked terms</span>
                      <CaretRight size={16} className="text-textMuted" />
                    </button>
                  </div>
                </div>
              )}

              {view.problems.map((p) => (
                <p key={p} className="px-1 text-[12.5px] text-textMuted">
                  {p}
                </p>
              ))}
            </div>
          )}

          {view && (page === 'poll' || page === 'prediction') && (
            <PollPredictionComposer
              inline
              broadcasterId={view.channel_id}
              channelLogin={view.channel_login}
              initialKind={page}
              onClose={() => setPage('home')}
              onOutcome={apply}
            />
          )}

          {view && page === 'blocked' && <BlockedTermsPage channelId={view.channel_id} />}

          {view && page !== 'home' && page !== 'poll' && page !== 'prediction' && page !== 'blocked' && (
            <div className="pt-1">
              <ManagerForm
                kind={page}
                view={view}
                queue={queue}
                onQueue={setQueue}
                onOutcome={apply}
                onDone={() => setPage('home')}
              />
            </div>
          )}
        </div>
      </div>
    </MobileSheet>
  );
};

const Switch: React.FC<{ on: boolean; disabled: boolean; label: string; onToggle: () => void }> = ({
  on,
  disabled,
  label,
  onToggle,
}) => (
  <button
    role="switch"
    aria-checked={on}
    aria-label={label}
    disabled={disabled}
    onClick={onToggle}
    className={`relative w-[46px] h-[28px] rounded-full shrink-0 transition-colors disabled:opacity-60 ${
      on ? 'bg-success' : 'bg-white/15'
    }`}
  >
    <span
      className={`absolute top-[2px] left-[2px] w-6 h-6 rounded-full bg-white shadow transition-transform ${
        on ? 'translate-x-[18px]' : ''
      }`}
    />
  </button>
);

const PillButton: React.FC<{ danger?: boolean; disabled: boolean; onClick: () => void; children: React.ReactNode }> = ({
  danger,
  disabled,
  onClick,
  children,
}) => (
  <button
    disabled={disabled}
    onClick={onClick}
    className={`sn-touch h-9 px-3.5 rounded-xl text-[14px] font-semibold active:opacity-70 disabled:opacity-50 ${
      danger ? 'bg-error/15 text-error' : 'bg-white/10 text-textPrimary'
    }`}
  >
    {children}
  </button>
);

export default ManageChannelSheet;
