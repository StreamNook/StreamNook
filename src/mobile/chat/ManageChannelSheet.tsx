// The phone's stream manager: one sheet behind the chat menu's "Manage
// channel" row, shown to the broadcaster and the channel's moderators. A page
// of cards (Stream, Chat modes, Moderation), each opening its form in the same
// sheet. Rust builds the view and runs every write (services/stream_manager.rs);
// the forms are the desktop's own in their sheet layout, so both apps say the
// same thing, and every result shows in place.
import React, { useState } from 'react';
import { CaretLeft } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { ManagerForm, LiveRow, TITLES, type ManagerPanelKind } from '../../components/chat/StreamManagerPanel';
import PollPredictionComposer from '../../components/chat/PollPredictionComposer';
import { ChatModesGroup } from '../../components/chat/ChatModesGroup';
import { GroupCard, MenuRow } from '../../components/chat/managerKit';
import { liveFromView, liveHeld, useManagerStore, type ChatModeKind } from '../../components/chat/managerStore';
import { useStreamManager, type ManagerSection } from '../../services/streamManager';
import { BlockedTermsPage } from './BlockedTermsPage';

type Page = 'home' | 'poll' | 'prediction' | 'blocked' | ManagerPanelKind;

interface Props {
  open: boolean;
  onClose: () => void;
  channelId: string;
  channelLogin: string;
}

export const ManageChannelSheet: React.FC<Props> = ({ open, onClose, channelId, channelLogin }) => {
  const { view, queue, setQueue, error } = useStreamManager(channelId, channelLogin, open);
  const storeVersion = useManagerStore();
  const [page, setPage] = useState<Page>('home');

  const close = () => {
    setPage('home');
    onClose();
  };
  const home = () => setPage('home');

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
  const moderates = view?.pages.some((p) => p.id === 'moderation') ?? false;

  const openSection = (id: ManagerSection) => {
    if (id === 'chat') return;
    setPage(id as Page);
  };

  const streamRows = view?.sections.filter((s) => s.id !== 'announce') ?? [];

  return (
    <MobileSheet open={open} onClose={close} maxHeightFraction={0.92} fill>
      <div className="flex flex-col h-full min-h-0">
        <div className="relative flex min-h-11 shrink-0 items-center justify-center px-2 py-1">
          {page !== 'home' && (
            <button
              onClick={home}
              className="sn-touch absolute left-1 flex items-center gap-0.5 px-2 text-[15px] text-accent active:opacity-70"
            >
              <CaretLeft size={18} weight="bold" />
              Back
            </button>
          )}
          <span className="flex flex-col items-center">
            <span className="text-[16px] font-semibold text-textPrimary">{title}</span>
            {/* Whose channel and your role here, under the title like the desktop window. */}
            {page === 'home' && view && (
              <span className="text-[12.5px] text-textMuted">{[view.channel_login, view.role_line].join(' · ')}</span>
            )}
          </span>
        </div>

        <div className="flex-1 min-h-0 overflow-y-auto px-3 pb-4 sn-manage-touch">
          {!view && <p className="px-1 py-6 text-center text-[14px] text-textMuted">{error ?? 'Loading…'}</p>}

          {view && page === 'home' && (
            <div className="flex flex-col gap-4 pt-1">
              {streamRows.length > 0 && (
                <GroupCard title="Stream" layout="sheet">
                  {streamRows.map((s) =>
                    (s.id === 'poll' && (view.poll || liveHeld(storeVersion, view.channel_id, 'poll'))) ||
                    (s.id === 'prediction' && (view.prediction || liveHeld(storeVersion, view.channel_id, 'prediction'))) ? (
                      <LiveRow key={s.id} view={view} layout="sheet" which={s.id} />
                    ) : (
                      <MenuRow
                        key={s.id}
                        layout="sheet"
                        label={s.label}
                        detail={s.detail}
                        trailing={s.badge ? <span className="text-[13px] tabular-nums text-textMuted">{s.badge}</span> : undefined}
                        onClick={() => openSection(s.id)}
                      />
                    ),
                  )}
                </GroupCard>
              )}

              {view.chat && (
                <ChatModesGroup
                  layout="sheet"
                  channelId={view.channel_id}
                  channelLogin={view.channel_login}
                  chat={view.chat}
                  live={liveFromView(view.chat.modes.map((m) => ({ kind: m.kind as ChatModeKind, on: m.on, value: m.value })))}
                />
              )}

              {(view.sections.some((s) => s.id === 'announce') || moderates) && (
                <GroupCard title="Moderation" layout="sheet">
                  {view.sections.some((s) => s.id === 'announce') && (
                    <MenuRow layout="sheet" label="Announce" onClick={() => setPage('announce')} />
                  )}
                  {moderates && <MenuRow layout="sheet" label="Blocked terms" onClick={() => setPage('blocked')} />}
                </GroupCard>
              )}

              {view.problems.map((p) => (
                <p key={p} className="px-1 text-[12.5px] text-textMuted">
                  {p}
                </p>
              ))}
            </div>
          )}

          {view && (page === 'poll' || page === 'prediction') && (
            <div className="pt-1">
              <PollPredictionComposer view={view} layout="sheet" initialKind={page} onClose={home} />
            </div>
          )}

          {view && page === 'blocked' && <BlockedTermsPage channelId={view.channel_id} />}

          {view && page !== 'home' && page !== 'poll' && page !== 'prediction' && page !== 'blocked' && (
            <div className="pt-1">
              <ManagerForm kind={page} layout="sheet" view={view} queue={queue} onQueue={setQueue} onClose={home} />
            </div>
          )}
        </div>
      </div>
    </MobileSheet>
  );
};

export default ManageChannelSheet;
