// The one Tools button beside the message box, and the menu it opens: what you
// can do TO the channel, as opposed to the message (that is the emote menu).
// Grouped cards, each shown only when it applies to you here:
//   Drops                    a drops campaign runs for this game and the drops
//                            plugin is installed
//   Your channel / Channel   the stream manager's rows (stream info, polls,
//                            predictions, raid, ad break, rewards queue): the
//                            ones Rust says your role here can use
//   Chat modes, Moderation,  you moderate this Twitch channel; Moderation
//   Your settings            carries the unban requests waiting, if any
//                            (the button's tooltip counts them)
// Every switch and button shows its result on its own row; nothing toasts.
// Open larger takes everything to the Manage channel window.
// ChatWidget decides which apply and renders the button only when at least
// one does, so the row never carries a button that opens an empty menu.
import React, { useEffect, useId, useRef, useState } from 'react';
import { BarChart3, ChevronLeft, Maximize2, Megaphone, Pencil, Pickaxe, Send, Settings, Shield, Trophy, Tv, Wrench } from 'lucide-react';
import { motion, AnimatePresence } from 'framer-motion';
import { SevenTVLogo } from '../emotesets/SevenTVLogo';
import { Tooltip } from '../ui/Tooltip';
import { Toggle } from '../ui/Toggle';
import { Logger } from '../../utils/logger';
import { useAppStore } from '../../stores/AppStore';
import { useFitAbove } from '../../hooks/useFitAbove';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import type { PageId, StreamManagerView } from '../../services/streamManager';
import { LiveRow, type ManagerPanelKind } from './StreamManagerPanel';
import { ChatModesGroup } from './ChatModesGroup';
import { ModerationGroup } from './ModerationGroup';
import BlockedTermsSheet from './BlockedTermsSheet';
import UnbanRequestsPage from './UnbanRequestsPage';
import { countLabel, useUnbanRequests } from './unbanRequests';
import { liveFromRoom, liveHeld, readTool, runTool, toolKey, useManagerStore, type RunAnswer } from './managerStore';
import { FADE_BOTH, GroupCard, IconButton, MenuRow, QUIET_PILL, easeFor, useEscape, useInstant } from './managerKit';
import { ChannelPointsIcon } from '../ChannelPointsIcon';

export interface DropsTool {
  gameName: string;
  /** The plugin is collecting drops for this game right now. */
  collecting: boolean;
  /** Drop progress is still loading. */
  busy: boolean;
  /** Start or stop collecting; answers in words for the row. */
  toggle: () => Promise<RunAnswer>;
}

interface ChannelToolsMenuProps {
  broadcasterId: string;
  /** Login of the channel this menu acts on. Passed in rather than read from
   *  the store because a MultiChat pane's channel is not the main window's. */
  channelLogin: string;
  /** Whether the viewer owns THIS channel. Same reason. */
  isBroadcaster: boolean;
  /** You moderate this Twitch channel: chat modes, moderation, your settings.
   *  Twitch only, because every row behind it is a Helix call. */
  canModerate: boolean;
  /** You manage this live Twitch channel (broadcaster or moderator): the
   *  stream manager rows, once `manager` has loaded. */
  canRunPolls: boolean;
  /** Open the poll / prediction builder above the message box. */
  onStartPoll: (kind: 'poll' | 'prediction') => void;
  /** A builder, panel or the window is open, so the button reads as in use. */
  pollBuilderOpen: boolean;
  /** Rust's stream manager view for this channel, null until read. */
  manager: StreamManagerView | null;
  /** Why the view could not be read, and reading it again. */
  managerError: string | null;
  onRetryManager: () => void;
  /** Open one of the stream manager's panels above the message box. */
  onOpenManager: (kind: ManagerPanelKind) => void;
  /** Open the Manage channel window, on `page` or the last one used here. */
  onOpenWindow: (page: PageId | null) => void;
  /** The menu opened or closed, so the host reads the view only when used. */
  onOpenChange?: (open: boolean) => void;
  /** Null when no drops campaign can be collected here. */
  drops: DropsTool | null;
  roomState: {
    followersOnly: number;
    slow: number;
    subsOnly: boolean;
    emoteOnly: boolean;
    r9k: boolean;
  };
}

const ROW_ICON: Record<string, React.ReactNode> = {
  info: <Pencil size={14} />,
  poll: <BarChart3 size={14} />,
  prediction: <Trophy size={14} />,
  raid: <Send size={14} />,
  ads: <Tv size={14} />,
  rewards: <ChannelPointsIcon size={14} />,
  announce: <Megaphone size={14} />,
};

const ChannelToolsMenu: React.FC<ChannelToolsMenuProps> = ({
  broadcasterId,
  channelLogin,
  isBroadcaster,
  canModerate,
  canRunPolls,
  onStartPoll,
  pollBuilderOpen,
  manager,
  managerError,
  onRetryManager,
  onOpenManager,
  onOpenWindow,
  onOpenChange,
  drops,
  roomState,
}) => {
  const storeVersion = useManagerStore();
  // Actions without a subscription (stable for the store's lifetime); the only
  // reactive settings read is the mod-logs flag, so subscribe to just that.
  const { updateSettings, openSettings, openEmoteSets } = useAppStore.getState();
  const showModLogs = useAppStore((s) => s.settings.show_mod_logs);
  const [isOpen, setIsOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  // Shrinks from the top in a short window instead of running past the chat
  // column, which would cut off the menu's header and first cards.
  const menuRef = useRef<HTMLDivElement>(null);
  useFitAbove(menuRef, isOpen, 560);
  const instant = useInstant();
  const uid = useId();
  // The Blocked terms and Unban requests pages take the menu's body in place
  // of the cards.
  const [page, setPage] = useState<'tools' | 'blocked' | 'unban'>('tools');
  // Live while the button shows, so the tooltip and the Moderation row can
  // count them without the menu open. Rust backs off on a refused read.
  const { rows: unbanRows, resolve: resolveUnban } = useUnbanRequests(
    broadcasterId,
    channelLogin,
    canModerate && !!broadcasterId && !!channelLogin,
  );

  useEffect(() => {
    onOpenChange?.(isOpen);
  }, [isOpen, onOpenChange]);

  const close = () => {
    setIsOpen(false);
    setPage('tools');
  };
  const markInside = useOutsidePress(isOpen, [containerRef], close);
  // Esc steps back out of a page, then closes; it never reaches the player.
  useEscape(isOpen, () => (page !== 'tools' ? setPage('tools') : close()));

  const openPanel = (kind: ManagerPanelKind) => {
    close();
    onOpenManager(kind);
  };
  const startPoll = (kind: 'poll' | 'prediction') => {
    close();
    onStartPoll(kind);
  };

  // The stream manager's rows; Announce sits under Moderation.
  const channelRows = manager ? manager.sections.filter((s) => s.id !== 'announce') : [];
  const canAnnounce = !!manager?.sections.some((s) => s.id === 'announce');
  const canExpand = !!manager && manager.pages.length > 0;

  const inUse = isOpen || pollBuilderOpen;
  const collecting = !!drops?.collecting;
  // Quiet on purpose: the wrench looks the same, only its words count them.
  const unbanWords = unbanRows.length > 0 ? countLabel(unbanRows.length) : null;
  const tooltip = [collecting ? `Tools (collecting drops for ${drops?.gameName})` : 'Tools', unbanWords]
    .filter(Boolean)
    .join(' · ');

  const dropsKey = toolKey(broadcasterId, 'drops');
  const dropsEntry = readTool(storeVersion, dropsKey);
  const dropsWanted = dropsEntry.draft as { on: boolean } | undefined;
  const dropsOn = dropsEntry.phase === 'working' && dropsWanted ? dropsWanted.on : collecting;

  return (
    // Not positioned on purpose: the menu anchors to the composer's own
    // `relative` block, so it spans the whole message box like the emote menu.
    <div className="static inline-flex flex-shrink-0 self-center" ref={containerRef} onPointerDownCapture={markInside}>
      <Tooltip content={tooltip} side="top">
        <button
          onClick={() => {
            if (isOpen) close();
            else setIsOpen(true);
          }}
          aria-label={unbanWords ? `Channel tools, ${unbanWords}` : 'Channel tools'}
          aria-expanded={isOpen}
          // Where focus goes back to when the Manage channel window closes.
          data-sn-tools-button={channelLogin.toLowerCase()}
          className={`sn-composer-tool relative flex items-center justify-center w-9 h-9 transition-colors duration-200 ${
            inUse ? 'text-accent-neon' : 'text-textSecondary hover:text-accent-neon'
          }`}
        >
          <Wrench size={18} strokeWidth={inUse ? 2.5 : 2} />
          {/* Drops are being collected: the one thing in here worth seeing
              without opening it. */}
          {collecting && <span className="absolute top-1.5 right-1.5 h-1.5 w-1.5 rounded-full bg-success" aria-hidden />}
        </button>
      </Tooltip>

      <AnimatePresence>
        {isOpen && (
          <motion.div
            ref={menuRef}
            role="dialog"
            aria-label="Channel tools"
            data-sn-owns-keys=""
            initial={{ opacity: 0, y: 10, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 10, scale: 0.97 }}
            transition={easeFor(instant)}
            // The glazed controls' menu body, like the chat header's flyouts.
            className="glass-flyout !p-0 absolute z-[60] bottom-full left-0 right-0 mb-2 max-h-[560px] flex flex-col overflow-hidden rounded-xl origin-bottom"
          >
            {/* The panels' header: icon and title, a step above the card labels. */}
            <div className="flex items-center gap-2 border-b border-textPrimary/[0.06] px-3.5 py-2.5">
              {page !== 'tools' ? (
                <button
                  onClick={() => setPage('tools')}
                  aria-label="Back to tools"
                  className="-ml-1 flex flex-1 items-center gap-1.5 text-sm font-semibold text-textPrimary transition-colors hover:text-textSecondary"
                >
                  <ChevronLeft size={15} strokeWidth={2.25} aria-hidden />
                  {page === 'blocked' ? 'Blocked terms' : 'Unban requests'}
                </button>
              ) : (
                <>
                  <Wrench size={14} className="text-textSecondary" aria-hidden />
                  <span className="flex-1 text-sm font-semibold text-textPrimary">Tools</span>
                </>
              )}
              {page === 'tools' && canExpand && (
                <IconButton
                  label="Open larger"
                  onClick={() => {
                    close();
                    onOpenWindow(null);
                  }}
                >
                  <Maximize2 size={13} />
                </IconButton>
              )}
            </div>

            {page === 'blocked' && canModerate ? (
              <BlockedTermsSheet key={broadcasterId} broadcasterId={broadcasterId} />
            ) : page === 'unban' && canModerate ? (
              <UnbanRequestsPage key={broadcasterId} rows={unbanRows} resolve={resolveUnban} />
            ) : (
              <div className="min-h-0 flex-1 space-y-3 overflow-y-auto custom-scrollbar px-2.5 pb-4 pt-3.5" style={FADE_BOTH}>
                {drops && (
                  <GroupCard title="Drops" layout="menu">
                    <MenuRow
                      layout="menu"
                      icon={<Pickaxe size={14} />}
                      label={`Collect drops for ${drops.gameName}`}
                      labelFor={`${uid}-drops`}
                      line={dropsEntry.phase === 'done' || dropsEntry.phase === 'failed' ? dropsEntry.line : null}
                      failed={dropsEntry.phase === 'failed'}
                      trailing={
                        <Toggle
                          id={`${uid}-drops`}
                          enabled={dropsOn}
                          ariaLabel={`Collect drops for ${drops.gameName}`}
                          onChange={() => {
                            if (dropsEntry.phase === 'working' || drops.busy) return;
                            void runTool(dropsKey, drops.toggle, { draft: { on: !dropsOn } });
                          }}
                        />
                      }
                    />
                  </GroupCard>
                )}

                {canRunPolls && manager && channelRows.length > 0 && (
                  <GroupCard title={isBroadcaster ? 'Your channel' : 'Channel'} layout="menu">
                    {channelRows.map((row) => {
                      // A running poll or prediction is its controls, not a builder.
                      // It stays after End or Refund until that result has shown.
                      if (
                        (row.id === 'poll' && (manager.poll || liveHeld(storeVersion, broadcasterId, 'poll'))) ||
                        (row.id === 'prediction' && (manager.prediction || liveHeld(storeVersion, broadcasterId, 'prediction')))
                      ) {
                        return <LiveRow key={row.id} view={manager} layout="menu" which={row.id} />;
                      }
                      return (
                        <MenuRow
                          key={row.id}
                          layout="menu"
                          icon={ROW_ICON[row.id]}
                          label={row.label}
                          detail={row.detail}
                          trailing={row.badge ? <span className="text-[12px] tabular-nums text-textSecondary">{row.badge}</span> : undefined}
                          onClick={() => {
                            if (row.id === 'poll' || row.id === 'prediction') startPoll(row.id);
                            else openPanel(row.id as ManagerPanelKind);
                          }}
                        />
                      );
                    })}
                  </GroupCard>
                )}

                {canModerate &&
                  (manager?.chat ? (
                    <ChatModesGroup
                      layout="menu"
                      channelId={broadcasterId}
                      channelLogin={channelLogin}
                      chat={manager.chat}
                      live={liveFromRoom(roomState)}
                    />
                  ) : (
                    // Never missing without a word: still reading, or why not.
                    <GroupCard title="Chat modes" layout="menu">
                      <MenuRow
                        layout="menu"
                        label={
                          !manager && !managerError
                            ? 'Reading the room…'
                            : (manager?.problems.find((p) => p.startsWith('Chat modes')) ?? managerError ?? 'Chat modes could not be read')
                        }
                        trailing={
                          (managerError || manager) && (
                            <button type="button" onClick={onRetryManager} className={`${QUIET_PILL} h-7 px-3 text-[12px]`}>
                              Try again
                            </button>
                          )
                        }
                      />
                    </GroupCard>
                  ))}

                {canModerate && (
                  <ModerationGroup
                    layout="menu"
                    channelId={broadcasterId}
                    channelLogin={channelLogin}
                    unbanCount={unbanRows.length}
                    onOpen={setPage}
                    onAnnounce={canAnnounce ? () => openPanel('announce') : undefined}
                  />
                )}

                {canModerate && (
                  <GroupCard title="Your settings" layout="menu">
                    <MenuRow
                      layout="menu"
                      icon={<Shield size={14} />}
                      label="Mod logs pane"
                      labelFor={`${uid}-modlogs`}
                      trailing={
                        <Toggle
                          id={`${uid}-modlogs`}
                          enabled={showModLogs ?? false}
                          ariaLabel="Mod logs pane"
                          onChange={async () => {
                            try {
                              // Read the full settings fresh at click time; only the
                              // mod-logs flag is subscribed above.
                              const settings = useAppStore.getState().settings;
                              await updateSettings({ ...settings, show_mod_logs: !settings.show_mod_logs });
                            } catch (e) {
                              Logger.error('[ChannelToolsMenu] Failed to update settings:', e);
                            }
                          }}
                        />
                      }
                    />
                    <MenuRow
                      layout="menu"
                      icon={<SevenTVLogo className="h-3.5 w-auto" />}
                      label="Manage 7TV emotes"
                      onClick={() => {
                        close();
                        openEmoteSets({ twitchId: broadcasterId, tab: 'emotes' });
                      }}
                    />
                    <MenuRow
                      layout="menu"
                      icon={<Settings size={14} />}
                      label="Mod log settings"
                      onClick={() => {
                        close();
                        openSettings('Moderation');
                      }}
                    />
                  </GroupCard>
                )}

                {manager?.problems.map((p) => (
                  <p key={p} className="px-2.5 text-[11.5px] text-textSecondary">
                    {p}
                  </p>
                ))}
              </div>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
};

export default ChannelToolsMenu;
