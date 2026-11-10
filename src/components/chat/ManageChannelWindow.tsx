// The Manage channel window: every stream manager tool in one centered,
// roomy window, a side list of the pages Rust says your role may use here
// (live marks in words), and each page the tool's own large layout. Opened
// from any panel's or the builder's Open larger, or the Tools menu's; what
// you typed comes along, and Collapse takes it back to the panel. Esc, a
// press outside and Close all close it.
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { motion } from 'framer-motion';
import { ChevronLeft, Minimize2, RotateCw, X } from 'lucide-react';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import type { PageId, QueueView, StreamManagerView } from '../../services/streamManager';
import { LivePage, ManagerForm, TITLES, type ManagerPanelKind } from './StreamManagerPanel';
import PollPredictionComposer from './PollPredictionComposer';
import { ChatModesGroup } from './ChatModesGroup';
import { ModerationGroup } from './ModerationGroup';
import BlockedTermsSheet from './BlockedTermsSheet';
import UnbanRequestsPage from './UnbanRequestsPage';
import { useUnbanRequests } from './unbanRequests';
import { collapseTarget, groupPages, initialPage, isPanelKind, lastPage, liveFromRoom, rememberPage } from './managerStore';
import { IconButton, LABEL, QUIET_PILL, easeFor, useEscape, useInstant } from './managerKit';

const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

interface Props {
  view: StreamManagerView | null;
  error: string | null;
  onRetry: () => void;
  channelId: string;
  channelLogin: string;
  /** The page asked for (an Open larger), or null for the last one used here. */
  requested: PageId | null;
  /** Opened from the other size with what was typed. */
  handedOff: boolean;
  queue: QueueView | null;
  onQueue: (channelId: string, q: QueueView) => void;
  /** The room's live modes (IRC ROOMSTATE), for Chat modes. */
  roomState: { followersOnly: number; slow: number; subsOnly: boolean; emoteOnly: boolean; r9k: boolean };
  /** Back to the panel or builder for `target`, with what was typed. */
  onCollapse: (target: ManagerPanelKind | 'poll' | 'prediction') => void;
  onClose: () => void;
}

export function ManageChannelWindow({
  view,
  error,
  onRetry,
  channelId,
  channelLogin,
  requested,
  handedOff,
  queue,
  onQueue,
  roomState,
  onCollapse,
  onClose,
}: Props) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const instant = useInstant();
  const [picked, setPicked] = useState<PageId | null>(() => requested);
  const [sub, setSub] = useState<'blocked' | 'unban' | null>(null);
  const [carried, setCarried] = useState(handedOff);
  const pages = view?.pages ?? [];
  // The same list the Tools menu counts (one read, then Rust's pushes).
  const unban = useUnbanRequests(channelId, channelLogin, pages.some((p) => p.id === 'moderation'));
  const page = view ? initialPage(picked, lastPage(channelId), pages) : null;

  useEffect(() => {
    if (page) rememberPage(channelId, page);
  }, [channelId, page]);

  // Focus moves in on open and on every page, and goes back where it was
  // (the Tools button, the panel's Open larger) when the window closes.
  useLayoutEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    return () => {
      if (before && document.contains(before)) before.focus();
    };
  }, []);
  useEffect(() => {
    const root = dialogRef.current?.querySelector<HTMLElement>('[data-sn-page]');
    if (!root || root.contains(document.activeElement)) return;
    const first = root.querySelector<HTMLElement>('input:not([disabled]), textarea:not([disabled]), button:not([disabled])');
    (first ?? dialogRef.current)?.focus();
  }, [page, sub]);

  const markInside = useOutsidePress(true, [dialogRef], onClose);
  useEscape(true, () => (sub ? setSub(null) : onClose()));

  const go = (p: PageId) => {
    setPicked(p);
    setSub(null);
    setCarried(false);
  };

  const trapTab = (e: ReactKeyboardEvent) => {
    if (e.key !== 'Tab' || !dialogRef.current) return;
    const items = Array.from(dialogRef.current.querySelectorAll<HTMLElement>(FOCUSABLE)).filter((el) => el.getClientRects().length > 0);
    if (!items.length) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  const row = pages.find((p) => p.id === page) ?? null;
  const collapse = view && page ? collapseTarget(page, view) : null;
  const close = () => onClose();

  let body: ReactNode = null;
  if (!view) {
    body = (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center">
        <p className="text-[13.5px] text-textSecondary">{error ?? 'Loading…'}</p>
        {error && (
          <button type="button" onClick={onRetry} className={`${QUIET_PILL} flex h-8 items-center gap-1.5 px-3.5 text-[13px]`}>
            <RotateCw size={13} aria-hidden />
            Try again
          </button>
        )}
      </div>
    );
  } else if (!page) {
    body = <p className="px-6 py-8 text-center text-[13.5px] text-textSecondary">{view.role_line}</p>;
  } else if (isPanelKind(page)) {
    body = (
      <ManagerForm
        key={page}
        kind={page}
        layout="window"
        view={view}
        queue={queue}
        onQueue={onQueue}
        onClose={close}
        handedOff={carried}
      />
    );
  } else if (page === 'poll' || page === 'prediction') {
    body = (page === 'poll' ? view.poll : view.prediction) ? (
      <LivePage view={view} which={page} />
    ) : (
      <PollPredictionComposer key="builder" view={view} layout="window" fixedKind={page} onClose={close} />
    );
  } else if (page === 'chat_modes') {
    body = (
      <div className="min-h-0 flex-1 overflow-y-auto scrollbar-thin px-5 pb-5 pt-1">
        {view.chat ? (
          <ChatModesGroup layout="window" channelId={channelId} channelLogin={channelLogin} chat={view.chat} live={liveFromRoom(roomState)} />
        ) : (
          <p className="text-[13px] text-textSecondary">{view.problems.find((p) => p.startsWith('Chat')) ?? 'Loading…'}</p>
        )}
      </div>
    );
  } else if (page === 'moderation') {
    body =
      sub === 'blocked' ? (
        <BlockedTermsSheet key={channelId} broadcasterId={channelId} />
      ) : sub === 'unban' ? (
        <UnbanRequestsPage key={channelId} rows={unban.rows} resolve={unban.resolve} />
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto scrollbar-thin px-5 pb-5 pt-1">
          <ModerationGroup layout="window" channelId={channelId} channelLogin={channelLogin} unbanCount={unban.rows.length} onOpen={setSub} />
        </div>
      );
  }

  const title = sub === 'blocked' ? 'Blocked terms' : sub === 'unban' ? 'Unban requests' : (row?.label ?? (page && isPanelKind(page) ? TITLES[page].title : 'Manage channel'));

  const nav = (
    <nav
      aria-label="Manage channel"
      className="flex flex-shrink-0 gap-1 overflow-x-auto border-b border-textPrimary/[0.06] px-3 py-2 min-[760px]:w-[208px] min-[760px]:flex-col min-[760px]:gap-3 min-[760px]:overflow-x-visible min-[760px]:overflow-y-auto min-[760px]:border-b-0 min-[760px]:border-r min-[760px]:px-2.5 min-[760px]:py-4"
    >
      {groupPages(pages).map((g) => (
        <div key={g.group} className="flex flex-shrink-0 gap-1 min-[760px]:flex-col min-[760px]:gap-0.5">
          <p className={`${LABEL} hidden px-2.5 pb-1 min-[760px]:block`}>{g.group}</p>
          {g.rows.map((p) => {
            const on = p.id === page;
            return (
              <button
                key={p.id}
                type="button"
                onClick={() => go(p.id)}
                aria-current={on ? 'page' : undefined}
                className={`flex flex-shrink-0 items-center gap-2 whitespace-nowrap rounded-lg px-2.5 py-1.5 text-left text-[13px] transition-colors ${
                  on ? 'glaze-inset bg-textPrimary/[0.1] font-semibold text-textPrimary' : 'text-textSecondary hover:bg-textPrimary/[0.05] hover:text-textPrimary'
                }`}
              >
                <span className="min-w-0 flex-1 truncate">{p.label}</span>
                {p.live && (
                  <span className="flex items-center gap-1 text-[11px] font-medium text-success">
                    <span className="h-1.5 w-1.5 rounded-full bg-success" aria-hidden />
                    {p.live}
                  </span>
                )}
                {p.badge && <span className="text-[11px] tabular-nums text-textSecondary">{p.badge}</span>}
              </button>
            );
          })}
        </div>
      ))}
    </nav>
  );

  return createPortal(
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={instant ? { duration: 0 } : { duration: 0.15 }}
      // Pinned from the top, so a list opening inside grows it downward
      // instead of re-centering it under the pointer.
      className="fixed inset-0 z-[calc(var(--sn-z-over-player-lift,0)_+_200)] flex items-start justify-center bg-black/50 px-4 pb-6 pt-[max(24px,calc(50vh-330px))]"
    >
      <motion.div
        ref={dialogRef}
        onPointerDownCapture={markInside}
        onKeyDown={trapTab}
        role="dialog"
        aria-modal
        aria-label="Manage channel"
        data-sn-owns-keys=""
        tabIndex={-1}
        initial={{ opacity: 0, y: 8, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={easeFor(instant)}
        className="glass-flyout flex h-[min(660px,90vh)] w-[min(960px,96vw)] flex-col overflow-hidden rounded-2xl !p-0 outline-none min-[760px]:flex-row"
      >
        {view && pages.length > 0 && nav}
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <div className="flex items-center gap-2 px-5 pb-3 pt-4">
            {sub && (
              <IconButton label="Back" onClick={() => setSub(null)} size="h-[30px] w-[30px]">
                <ChevronLeft size={15} />
              </IconButton>
            )}
            <div className="min-w-0 flex-1">
              <p className="truncate text-[16px] font-semibold leading-tight text-textPrimary">{title}</p>
              {view && <p className="truncate text-[12.5px] text-textSecondary">{[view.channel_login, view.role_line].join(' · ')}</p>}
            </div>
            {/* On the title's line, not centered on title and subtitle. */}
            <div className="-mt-[5px] flex gap-2 self-start">
              {collapse && !sub && (
                <IconButton label="Back to the panel" onClick={() => onCollapse(collapse)} size="h-[30px] w-[30px]">
                  <Minimize2 size={14} />
                </IconButton>
              )}
              <IconButton label="Close" onClick={close} size="h-[30px] w-[30px]">
                <X size={15} />
              </IconButton>
            </div>
          </div>
          <div data-sn-page className="flex min-h-0 flex-1 flex-col">
            {body}
          </div>
        </div>
      </motion.div>
    </motion.div>,
    document.body,
  );
}

export default ManageChannelWindow;
