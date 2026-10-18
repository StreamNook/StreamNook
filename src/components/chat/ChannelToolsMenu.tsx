// The one Tools button beside the message box, and the menu it opens: what you
// can do TO the channel, as opposed to the message (that is the emote menu).
// Each section shows only when it applies to you here:
//   Drops                    a drops campaign runs for this game and the drops
//                            plugin is installed
//   Your channel / Channel   the stream manager's rows (stream info, polls,
//                            predictions, raid, ad break, rewards queue): the
//                            ones Rust says your role here can use
//   Chat modes, Moderation,  you moderate this Twitch channel
//   Your settings
// ChatWidget decides which apply and renders the button only when at least
// one does, so the row never carries a button that opens an empty menu.
import React, { useState, useRef, useEffect } from 'react';
import { Shield, ShieldAlert, Users, MessageSquare, Clock, Slash, Hash, Settings, ChevronRight, Zap, PinOff, Ban, BarChart3, Trophy, Pickaxe, Wrench, Pencil, Send, Tv, Megaphone } from 'lucide-react';
import { ChannelPointsIcon } from '../ChannelPointsIcon';
import { SevenTVLogo } from '../emotesets/SevenTVLogo';
import { motion, AnimatePresence } from 'framer-motion';
import { Tooltip } from '../ui/Tooltip';
import { GlassSelect } from '../ui/GlassSelect';
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../../utils/logger';
import { useAppStore } from '../../stores/AppStore';
import { usePinStore } from '../../stores/pinStore';
import { isImeComposing } from '../../utils/imeComposing';
import { useFitAbove } from '../../hooks/useFitAbove';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { managerAct, type ActOutcome, type ManagerAction, type SectionRow, type StreamManagerView } from '../../services/streamManager';
import type { ManagerPanelKind } from './StreamManagerPanel';

export interface DropsTool {
  gameName: string;
  /** The plugin is collecting drops for this game right now. */
  collecting: boolean;
  /** A start or stop is in flight. */
  busy: boolean;
  onToggle: () => void;
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
  /** A builder is open, so the button reads as in use. */
  pollBuilderOpen: boolean;
  /** Rust's stream manager view for this channel, null until read. */
  manager: StreamManagerView | null;
  /** Open one of the stream manager's builders above the message box. */
  onOpenManager: (kind: ManagerPanelKind) => void;
  /** A write answered; the host keeps its view current from it. */
  onManagerOutcome: (outcome: ActOutcome) => void;
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

const SECTION_LABEL = 'text-[10px] font-semibold text-white/40 tracking-wider uppercase';

const ChannelToolsMenu: React.FC<ChannelToolsMenuProps> = ({
  broadcasterId,
  channelLogin,
  isBroadcaster,
  canModerate,
  canRunPolls,
  onStartPoll,
  pollBuilderOpen,
  manager,
  onOpenManager,
  onManagerOutcome,
  onOpenChange,
  drops,
  roomState,
}) => {
  // Actions without a subscription (stable for the store's lifetime); the only
  // reactive settings read is the mod-logs flag, so subscribe to just that.
  const { updateSettings, openSettings, openEmoteSets, addToast } = useAppStore.getState();
  const showModLogs = useAppStore((s) => s.settings.show_mod_logs);
  const [isOpen, setIsOpen] = useState(false);
  const [isConfirmingClear, setIsConfirmingClear] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  // Shrinks from the top in a short window instead of running past the chat
  // column, which would cut off the menu's header and first sections.
  const menuRef = useRef<HTMLDivElement>(null);
  useFitAbove(menuRef, isOpen, 520);
  const [isPending, setIsPending] = useState(false);
  const [blockTerm, setBlockTerm] = useState('');
  // Read once per open rather than subscribed: the engine holds this in a plain
  // module Map with no store behind it, and the menu is short-lived anyway.
  const [activeNukeCount, setActiveNukeCount] = useState(0);

  // Shield Mode clears chat and tightens the modes by Twitch's defaults, so
  // turning it on asks first, like Clear chat.
  const [isConfirmingShield, setIsConfirmingShield] = useState(false);

  useEffect(() => {
    onOpenChange?.(isOpen);
    if (!isOpen || !channelLogin) return;
    let alive = true;
    if (canModerate) {
      void invoke<number>('nuke_active', { channel: channelLogin })
        .then((n) => alive && setActiveNukeCount(n))
        .catch(() => {});
    }
    return () => {
      alive = false;
    };
  }, [isOpen, channelLogin, canModerate, onOpenChange]);

  // The running poll and prediction controls and every stream manager write go
  // through Rust's one door, which picks Helix for the broadcaster and Twitch's
  // own GQL for moderators and editors.
  const runManagerAction = async (action: ManagerAction) => {
    if (isPending || !manager) return;
    setIsPending(true);
    try {
      const outcome = await managerAct(manager.channel_id, manager.channel_login, action);
      onManagerOutcome(outcome);
      addToast(outcome.message, outcome.ok ? 'success' : 'error');
    } catch (err) {
      Logger.error('[ChannelToolsMenu] stream manager action failed:', err);
      addToast(typeof err === 'string' ? err : 'That did not go through', 'error');
    } finally {
      setIsPending(false);
    }
  };

  const openPanel = (kind: ManagerPanelKind) => {
    setIsOpen(false);
    onOpenManager(kind);
  };

  const markInside = useOutsidePress(isOpen, [containerRef], () => {
    setIsOpen(false);
    setIsConfirmingClear(false);
    setIsConfirmingShield(false);
  });

  const handleToggleSetting = async (setting: string, currentValue: boolean | number) => {
    if (isPending || !broadcasterId) return;
    setIsPending(true);

    try {
      const payload: Record<string, boolean | number> = {};

      switch (setting) {
        case 'emote_mode':
          payload.emote_mode = !currentValue;
          break;
        case 'follower_mode':
          payload.follower_mode = (currentValue === -1);
          // Standardized default to 0 (all followers) if turning on
          if (currentValue === -1) payload.follower_mode_duration = 0;
          break;
        case 'subscriber_mode':
          payload.subscriber_mode = !currentValue;
          break;
        case 'slow_mode':
          payload.slow_mode = (currentValue === 0);
          if (currentValue === 0) payload.slow_mode_wait_time = 10; // Default 10s slow mode
          break;
        case 'unique_chat_mode':
          payload.unique_chat_mode = !currentValue;
          break;
      }

      await invoke('update_chat_settings', {
        broadcasterId,
        settings: payload
      });
      Logger.debug(`[ChannelToolsMenu] Updated chat setting ${setting}:`, payload);

    } catch (err) {
      Logger.error(`[ChannelToolsMenu] Failed to update chat setting ${setting}:`, err);
    } finally {
      setIsPending(false);
    }
  };

  const handleClearChat = async () => {
    if (isPending || !broadcasterId) return;
    setIsPending(true);

    try {
      await invoke('clear_chat', { broadcasterId });
      Logger.debug('[ChannelToolsMenu] Cleared chat successfully.');
      setIsConfirmingClear(false);
      setIsOpen(false);
    } catch (err) {
      Logger.error('[ChannelToolsMenu] Failed to clear chat:', err);
    } finally {
      setIsPending(false);
    }
  };

  const startPoll = (kind: 'poll' | 'prediction') => {
    setIsOpen(false);
    onStartPoll(kind);
  };

  // The stream manager's rows; Announce sits under Moderation. Until Rust's
  // view arrives the broadcaster keeps the two builders it always had.
  const channelRows: SectionRow[] = manager
    ? manager.sections.filter((s) => s.id !== 'announce')
    : isBroadcaster
      ? [
          { id: 'poll', label: 'Start a poll', detail: null, badge: null },
          { id: 'prediction', label: 'Start a prediction', detail: null, badge: null },
        ]
      : [];
  const canAnnounce = !!manager?.sections.some((s) => s.id === 'announce');
  const shieldOn = manager?.chat?.shield ?? null;

  const inUse = isOpen || pollBuilderOpen;
  const collecting = !!drops?.collecting;

  return (
    // Not positioned on purpose: the menu anchors to the composer's own
    // `relative` block, so it spans the whole message box like the emote menu.
    <div className="static inline-flex flex-shrink-0 self-center" ref={containerRef} onPointerDownCapture={markInside}>
      <Tooltip content={collecting ? `Tools (collecting drops for ${drops?.gameName})` : 'Tools'} side="top">
        <button
          onClick={() => {
            setIsOpen(!isOpen);
            setIsConfirmingClear(false);
          }}
          aria-label="Channel tools"
          aria-expanded={isOpen}
          className={`sn-composer-tool relative flex items-center justify-center w-9 h-9 transition-colors duration-200 ${
            inUse ? 'text-accent-neon' : 'text-textSecondary hover:text-accent-neon'
          }`}
        >
          <Wrench size={18} strokeWidth={inUse ? 2.5 : 2} />
          {/* Drops are being collected: the one thing in here worth seeing
              without opening it. */}
          {collecting && (
            <span className="absolute top-1.5 right-1.5 h-1.5 w-1.5 rounded-full bg-success" aria-hidden />
          )}
        </button>
      </Tooltip>

      <AnimatePresence>
        {isOpen && (
          <motion.div
            ref={menuRef}
            initial={{ opacity: 0, y: 10, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 10, scale: 0.95 }}
            transition={{ duration: 0.15, ease: "easeOut" }}
            className="sn-popover absolute z-[60] bottom-full left-0 right-0 mb-2 max-h-[520px] flex flex-col overflow-hidden origin-bottom"
          >
            <div className="px-4 py-3 border-b border-white/5 flex items-center bg-glass-base/50 backdrop-blur-md shadow-sm z-10 relative">
              <span className="text-[11px] font-semibold text-white/50 tracking-wider uppercase">Tools</span>
            </div>

            <div className="flex-1 min-h-0 overflow-y-auto custom-scrollbar p-1">
              {drops && (
                <div className="p-1.5 flex flex-col gap-1">
                  <div className="px-2 pt-1 pb-0.5">
                    <span className={SECTION_LABEL}>Drops</span>
                  </div>
                  <MenuToggleItem
                    icon={<Pickaxe size={14} />}
                    label={`Collect drops for ${drops.gameName}`}
                    isActive={drops.collecting}
                    onClick={drops.onToggle}
                    disabled={drops.busy}
                  />
                </div>
              )}

              {canRunPolls && channelRows.length > 0 && (
                <div className={`p-1.5 flex flex-col gap-1 ${drops ? 'border-t border-white/10' : ''}`}>
                  <div className="px-2 pt-1 pb-0.5">
                    <span className={SECTION_LABEL}>{isBroadcaster ? 'Your channel' : 'Channel'}</span>
                  </div>
                  {channelRows.map((row) => {
                    // A running poll or prediction is its controls, not a
                    // builder: End keeps the result on screen, Remove hides it.
                    if (row.id === 'poll' && manager?.poll) {
                      return (
                        <LiveRow key="poll" icon={<BarChart3 size={14} />} label={row.label} detail={row.detail}>
                          <LiveButton disabled={isPending} onClick={() => runManagerAction({ kind: 'end_poll', show_results: true })}>
                            End
                          </LiveButton>
                          <LiveButton danger disabled={isPending} onClick={() => runManagerAction({ kind: 'end_poll', show_results: false })}>
                            Remove
                          </LiveButton>
                        </LiveRow>
                      );
                    }
                    if (row.id === 'prediction' && manager?.prediction) {
                      const p = manager.prediction;
                      return (
                        <LiveRow key="prediction" icon={<Trophy size={14} />} label={p.status === 'locked' ? 'Prediction locked' : row.label} detail={row.detail}>
                          {p.status === 'active' && (
                            <LiveButton disabled={isPending} onClick={() => runManagerAction({ kind: 'lock_prediction' })}>
                              Lock
                            </LiveButton>
                          )}
                          <LiveButton danger disabled={isPending} onClick={() => runManagerAction({ kind: 'cancel_prediction' })}>
                            Refund
                          </LiveButton>
                        </LiveRow>
                      );
                    }
                    return (
                      <MenuActionItem
                        key={row.id}
                        icon={ROW_ICON[row.id]}
                        label={row.label}
                        detail={row.detail}
                        badge={row.badge}
                        onClick={() => {
                          if (row.id === 'poll' || row.id === 'prediction') startPoll(row.id);
                          else openPanel(row.id as ManagerPanelKind);
                        }}
                      />
                    );
                  })}
                </div>
              )}

              {canModerate && (
                <>
                  <div className={`p-1.5 flex flex-col gap-1 ${drops || canRunPolls ? 'border-t border-white/10' : ''}`}>
                    <div className="px-2 pt-1 pb-0.5">
                      <span className={SECTION_LABEL}>Chat modes</span>
                    </div>
                    <MenuToggleItem
                      icon={<MessageSquare size={14} />}
                      label="Emote-only"
                      isActive={roomState.emoteOnly}
                      onClick={() => handleToggleSetting('emote_mode', roomState.emoteOnly)}
                      disabled={isPending}
                    />
                    <MenuToggleItem
                      icon={<Users size={14} />}
                      label="Followers-only"
                      isActive={roomState.followersOnly !== -1}
                      onClick={() => handleToggleSetting('follower_mode', roomState.followersOnly)}
                      disabled={isPending}
                    />
                    <MenuToggleItem
                      icon={<Shield size={14} />}
                      label="Subscriber-only"
                      isActive={roomState.subsOnly}
                      onClick={() => handleToggleSetting('subscriber_mode', roomState.subsOnly)}
                      disabled={isPending}
                    />
                    <div className="px-2.5 py-2 flex items-center justify-between hover:bg-white/5 rounded-md transition-colors">
                      <div className="flex items-center gap-2.5">
                        <div className={`flex items-center justify-center ${roomState.slow > 0 ? 'text-green-400' : 'text-white/40'}`}>
                          <Clock size={14} />
                        </div>
                        <span className={`text-sm ${roomState.slow > 0 ? 'text-white' : 'text-white/70'}`}>
                          Slow mode
                        </span>
                      </div>
                      <div className="w-[85px] z-[60]" onClick={e => e.stopPropagation()}>
                          <GlassSelect
                            value={roomState.slow > 0 ? roomState.slow.toString() : '0'}
                            onChange={(val) => {
                              const numericVal = parseInt(val, 10);
                              if (isPending || !broadcasterId) return;
                              setIsPending(true);
                              invoke('update_chat_settings', {
                                broadcasterId,
                                settings: { slow_mode: numericVal > 0, slow_mode_wait_time: numericVal > 0 ? numericVal : undefined }
                              }).then(() => {
                                 Logger.debug(`[ChannelToolsMenu] Updated slow mode to ${numericVal}s`);
                              }).catch(err => {
                                 Logger.error(`[ChannelToolsMenu] Failed to update slow mode:`, err);
                              }).finally(() => {
                                 setIsPending(false);
                              });
                            }}
                            options={[
                              { value: '0', label: 'Off' },
                              { value: '3', label: '3s' },
                              { value: '5', label: '5s' },
                              { value: '10', label: '10s' },
                              { value: '20', label: '20s' },
                              { value: '30', label: '30s' },
                              { value: '60', label: '60s' },
                              { value: '120', label: '120s' },
                            ]}
                            placement="top"
                            className="!py-1.5 !px-2.5 !min-w-[85px] !text-[12px] !bg-white/5 hover:!bg-white/10"
                          />
                      </div>
                    </div>
                    <MenuToggleItem
                      icon={<Hash size={14} />}
                      label="Unique chat"
                      isActive={roomState.r9k}
                      onClick={() => handleToggleSetting('unique_chat_mode', roomState.r9k)}
                      disabled={isPending}
                    />
                    {shieldOn !== null &&
                      (isConfirmingShield ? (
                        <div className="px-2.5 py-2 flex items-center justify-between gap-2">
                          <span className="text-xs text-white/70">Turn on Shield Mode? It can clear chat.</span>
                          <div className="flex items-center gap-1.5 flex-shrink-0">
                            <LiveButton
                              danger
                              disabled={isPending}
                              onClick={() => {
                                setIsConfirmingShield(false);
                                void runManagerAction({ kind: 'shield', on: true });
                              }}
                            >
                              Turn on
                            </LiveButton>
                            <LiveButton disabled={isPending} onClick={() => setIsConfirmingShield(false)}>
                              No
                            </LiveButton>
                          </div>
                        </div>
                      ) : (
                        <MenuToggleItem
                          icon={<ShieldAlert size={14} />}
                          label="Shield Mode"
                          isActive={shieldOn}
                          onClick={() =>
                            shieldOn ? void runManagerAction({ kind: 'shield', on: false }) : setIsConfirmingShield(true)
                          }
                          disabled={isPending}
                        />
                      ))}
                  </div>

                  <div className="p-1.5 border-t border-white/10 flex flex-col gap-1">
                    <div className="px-2 pt-1 pb-0.5">
                      <span className={SECTION_LABEL}>Moderation</span>
                    </div>

                    {canAnnounce && (
                      <MenuActionItem icon={<Megaphone size={14} />} label="Announce" onClick={() => openPanel('announce')} />
                    )}

                    {/* Only rendered while a /nuke is still watching new messages, so
                        the menu doesn't carry a dead row the rest of the time. */}
                    {activeNukeCount > 0 && (
                      <button
                        onClick={async () => {
                          const stopped = await invoke<number>('nuke_stop', { channel: channelLogin }).catch(() => 0);
                          setActiveNukeCount(0);
                          addToast(`Stopped ${stopped} running nuke window(s)`, 'success');
                        }}
                        className="w-full text-left px-2.5 py-2 rounded-md flex items-center justify-between hover:bg-warning/10 transition-colors group"
                      >
                        <div className="flex items-center gap-2.5">
                          <div className="flex items-center justify-center text-warning">
                            <Zap size={14} />
                          </div>
                          <span className="text-sm text-white/70 group-hover:text-white">
                            Stop running nuke
                          </span>
                        </div>
                        <span className="text-[11px] tabular-nums text-warning">{activeNukeCount}</span>
                      </button>
                    )}

                    <button
                      onClick={async () => {
                        if (isPending || !broadcasterId) return;
                        setIsPending(true);
                        try {
                          await invoke('unpin_chat_message', { broadcasterId });
                          usePinStore.getState().requestRefresh();
                          addToast('Message unpinned', 'success');
                        } catch (err) {
                          Logger.error('[ChannelToolsMenu] Failed to unpin:', err);
                          addToast(typeof err === 'string' ? err : 'Nothing pinned right now', 'error');
                        } finally {
                          setIsPending(false);
                        }
                      }}
                      disabled={isPending}
                      className="w-full text-left px-2.5 py-2 rounded-md flex items-center gap-2.5 hover:bg-white/5 transition-colors group disabled:opacity-50"
                    >
                      <div className="flex items-center justify-center text-white/40 group-hover:text-accent transition-colors">
                        <PinOff size={14} />
                      </div>
                      <span className="text-sm text-white/70 group-hover:text-white">Unpin message</span>
                    </button>

                    {/* Blocked terms. An inline field rather than a sub-panel: adding
                        one is the whole interaction, and AutoMod's full list already
                        lives on Twitch. */}
                    <div className="px-2.5 py-2 flex items-center gap-2.5">
                      <div className="flex items-center justify-center text-white/40 flex-shrink-0">
                        <Ban size={14} />
                      </div>
                      <input
                        value={blockTerm}
                        onChange={(e) => setBlockTerm(e.target.value)}
                        onKeyDown={async (e) => {
                          if (e.key !== 'Enter' || isImeComposing(e)) return;
                          const phrase = blockTerm.trim();
                          if (!phrase || isPending || !broadcasterId) return;
                          setIsPending(true);
                          try {
                            await invoke('add_blocked_term', { broadcasterId, text: phrase });
                            addToast(`Blocked "${phrase}"`, 'success');
                            setBlockTerm('');
                          } catch (err) {
                            Logger.error('[ChannelToolsMenu] Failed to block term:', err);
                            addToast(typeof err === 'string' ? err : 'Could not block that', 'error');
                          } finally {
                            setIsPending(false);
                          }
                        }}
                        placeholder="Block a phrase, then Enter"
                        className="flex-1 min-w-0 bg-white/5 hover:bg-white/10 focus:bg-white/10 rounded px-2 py-1 text-[12px] text-white/90 placeholder-white/30 outline-none transition-colors"
                      />
                    </div>
                  </div>

                  <div className="p-1.5 border-t border-white/10 flex flex-col gap-1">
                    <div className="px-2 pt-1 pb-0.5">
                      <span className={SECTION_LABEL}>Your settings</span>
                    </div>
                    <MenuToggleItem
                      icon={<Shield size={14} className="text-accent" />}
                      label="Mod logs pane"
                      isActive={showModLogs ?? false}
                      onClick={async () => {
                        try {
                          // Read the full settings fresh at click time; only the
                          // mod-logs flag is subscribed above.
                          const settings = useAppStore.getState().settings;
                          await updateSettings({ ...settings, show_mod_logs: !settings.show_mod_logs });
                        } catch (e) {
                          Logger.error('[ChannelToolsMenu] Failed to update settings:', e);
                        }
                      }}
                      disabled={false}
                    />
                    <button
                      onClick={() => { openEmoteSets({ twitchId: broadcasterId, tab: 'emotes' }); setIsOpen(false); }}
                      className="w-full text-left px-2.5 py-2 rounded-md flex items-center justify-between hover:bg-white/5 transition-colors group"
                    >
                      <div className="flex items-center gap-2.5">
                        <div className="flex items-center justify-center text-white/40 group-hover:text-[#29b6f6] transition-colors">
                          <SevenTVLogo className="h-3.5 w-auto" />
                        </div>
                        <span className="text-sm text-white/70 group-hover:text-white">Manage 7TV emotes</span>
                      </div>
                      <ChevronRight size={14} className="text-white/30 group-hover:text-white/60" />
                    </button>
                    <button
                      onClick={() => { openSettings('Moderation'); setIsOpen(false); }}
                      className="w-full text-left px-2.5 py-2 rounded-md flex items-center justify-between hover:bg-white/5 transition-colors group"
                    >
                      <div className="flex items-center gap-2.5">
                        <div className="flex items-center justify-center text-white/40 group-hover:text-accent transition-colors">
                          <Settings size={14} />
                        </div>
                        <span className="text-sm text-white/70 group-hover:text-white">Mod log settings</span>
                      </div>
                      <ChevronRight size={14} className="text-white/30 group-hover:text-white/60" />
                    </button>
                  </div>
                </>
              )}
            </div>

            {canModerate && (
              <div className="p-1.5 border-t border-white/10 shrink-0 bg-background/50">
                {!isConfirmingClear ? (
                  <button
                    onClick={() => setIsConfirmingClear(true)}
                    disabled={isPending}
                    className="w-full text-left px-2 py-1.5 rounded-md flex items-center gap-2.5 text-red-400 hover:bg-red-500/10 transition-colors text-sm"
                  >
                    <Slash size={14} />
                    <span>Clear chat</span>
                  </button>
                ) : (
                  <div className="flex items-center justify-between px-2 py-1">
                    <span className="text-xs text-red-400 font-medium tracking-wide">Clear everyone's chat?</span>
                    <div className="flex items-center gap-1.5">
                      <button
                        onClick={handleClearChat}
                        disabled={isPending}
                        className="px-2.5 py-1 text-xs font-semibold bg-red-500/20 hover:bg-red-500/30 text-red-400 rounded transition-colors disabled:opacity-50"
                      >
                        Yes
                      </button>
                      <button
                        onClick={() => setIsConfirmingClear(false)}
                        disabled={isPending}
                        className="px-2.5 py-1 text-xs font-semibold bg-white/10 hover:bg-white/20 text-white/70 rounded transition-colors disabled:opacity-50"
                      >
                        No
                      </button>
                    </div>
                  </div>
                )}
              </div>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
};


const MenuToggleItem = ({
  icon,
  label,
  isActive,
  onClick,
  disabled
}: {
  icon: React.ReactNode;
  label: string;
  isActive: boolean;
  onClick: () => void;
  disabled: boolean;
}) => {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      role="switch"
      aria-checked={isActive}
      className={`
        w-full text-left px-2.5 py-2 rounded-md flex items-center justify-between gap-2 transition-colors outline-none
        ${disabled ? 'opacity-50 cursor-not-allowed' : 'hover:bg-white/5 focus:bg-white/5'}
      `}
    >
      <div className="flex items-center gap-2.5 min-w-0">
        <div className={`
          flex items-center justify-center flex-shrink-0
          ${isActive ? 'text-green-400' : 'text-white/40'}
        `}>
          {icon}
        </div>
        <span className={`text-sm truncate ${isActive ? 'text-white' : 'text-white/70'}`}>
          {label}
        </span>
      </div>

      {/* Toggle switch visual */}
      <div className={`
        w-7 h-4 rounded-full flex-shrink-0 flex items-center p-0.5 transition-colors duration-200
        ${isActive ? 'bg-green-500' : 'bg-white/20'}
      `}>
        <div className={`
          w-3 h-3 rounded-full bg-white shadow-sm transition-transform duration-200
          ${isActive ? 'translate-x-3' : 'translate-x-0'}
        `} />
      </div>
    </button>
  );
};

const ROW_ICON: Record<string, React.ReactNode> = {
  info: <Pencil size={14} />,
  poll: <BarChart3 size={14} />,
  prediction: <Trophy size={14} />,
  raid: <Send size={14} />,
  ads: <Tv size={14} />,
  rewards: <ChannelPointsIcon size={14} />,
};

/** A row that does something once (opens a builder), rather than a switch.
 *  `detail` is a quieter second line; `badge` a short trailing count. */
const MenuActionItem = ({
  icon,
  label,
  detail,
  badge,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  detail?: string | null;
  badge?: string | null;
  onClick: () => void;
}) => (
  <button
    onClick={onClick}
    className="w-full text-left px-2.5 py-2 rounded-md flex items-center justify-between gap-2 hover:bg-white/5 transition-colors group"
  >
    <div className="flex items-center gap-2.5 min-w-0">
      <div className="flex items-center justify-center flex-shrink-0 text-white/40 group-hover:text-accent transition-colors">
        {icon}
      </div>
      <div className="min-w-0">
        <span className="block text-sm text-white/70 group-hover:text-white truncate">{label}</span>
        {detail && <span className="block text-[11px] text-white/40 truncate">{detail}</span>}
      </div>
    </div>
    <div className="flex items-center gap-1.5 flex-shrink-0">
      {badge && <span className="text-[11px] tabular-nums text-white/50">{badge}</span>}
      <ChevronRight size={14} className="text-white/30 group-hover:text-white/60" />
    </div>
  </button>
);

/** A running poll or prediction: its name, then its controls. */
const LiveRow = ({
  icon,
  label,
  detail,
  children,
}: {
  icon: React.ReactNode;
  label: string;
  detail?: string | null;
  children: React.ReactNode;
}) => (
  <div className="px-2.5 py-2 flex items-center justify-between gap-2">
    <div className="flex items-center gap-2.5 min-w-0">
      <div className="flex items-center justify-center text-accent flex-shrink-0">{icon}</div>
      <div className="min-w-0">
        <span className="block text-sm text-white/70 truncate">{label}</span>
        {detail && <span className="block text-[11px] text-white/40 truncate">{detail}</span>}
      </div>
    </div>
    <div className="flex items-center gap-1.5 flex-shrink-0">{children}</div>
  </div>
);

const LiveButton = ({
  danger,
  disabled,
  onClick,
  children,
}: {
  danger?: boolean;
  disabled: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) => (
  <button
    disabled={disabled}
    onClick={onClick}
    className={`px-2 py-1 text-[11px] font-semibold rounded transition-colors disabled:opacity-50 ${
      danger ? 'bg-red-500/15 hover:bg-red-500/25 text-red-300' : 'bg-white/10 hover:bg-white/20 text-white/80'
    }`}
  >
    {children}
  </button>
);

export default ChannelToolsMenu;
