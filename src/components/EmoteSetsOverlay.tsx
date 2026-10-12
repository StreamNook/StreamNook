// "Emote Sets" dashboard: manage 7TV emotes, sets, and editors for every
// channel the signed-in 7TV account can edit, so there's no need to leave the
// app for routine emote work. Opened from the command palette
// (`openEmoteSets`) or contextually from the moderator menu (pre-selected to a
// channel by Twitch id). Self-contained like DropsOverlay: reads its own store
// flag.
import { useCallback, useEffect, useMemo, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { X, Loader2, RefreshCw, Crown, Check, XCircle, ShieldAlert } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import { Tooltip } from './ui/Tooltip';
import { SevenTVLogo } from './emotesets/SevenTVLogo';
import {
  getEditableChannels,
  invalidateEditableChannels,
  respondToInvite,
  SevenTVSessionExpired,
  type EditableChannel,
} from '../services/seventvEditorService';
import ChannelWorkspace from './emotesets/ChannelWorkspace';
import { SevenTvAvatar } from './SevenTvAvatar';

type WorkspaceTab = 'emotes' | 'sets' | 'editors';

export default function EmoteSetsOverlay() {
  const showEmoteSetsOverlay = useAppStore((s) => s.showEmoteSetsOverlay);
  const setShow = useAppStore((s) => s.setShowEmoteSetsOverlay);
  const initialTwitchId = useAppStore((s) => s.emoteSetsOverlayInitialTwitchId);
  const initialTab = useAppStore((s) => s.emoteSetsOverlayInitialTab);
  const addToast = useAppStore((s) => s.addToast);

  const [loading, setLoading] = useState(true);
  const [connected, setConnected] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [channels, setChannels] = useState<EditableChannel[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const close = useCallback(() => setShow(false), [setShow]);

  const mySeventvId = useMemo(
    () => channels.find((c) => c.isSelf)?.seventvUserId,
    [channels],
  );

  const load = useCallback(
    async (preferTwitchId?: string | null) => {
      setLoading(true);
      setLoadError(null);
      try {
        const status = (await invoke('get_seventv_auth_status')) as { is_authenticated: boolean };
        if (!status?.is_authenticated) {
          setConnected(false);
          setChannels([]);
          return;
        }
        setConnected(true);
        const list = await getEditableChannels(undefined, true);
        setChannels(list);
        // Pick the contextual channel, else keep the current selection, else self.
        setSelectedId((prev) => {
          if (preferTwitchId) {
            const hit = list.find((c) => c.twitchId === preferTwitchId && c.inviteState === 'ACCEPTED');
            if (hit) return hit.seventvUserId;
          }
          if (prev && list.some((c) => c.seventvUserId === prev)) return prev;
          const firstUsable = list.find((c) => c.inviteState === 'ACCEPTED');
          return firstUsable?.seventvUserId ?? null;
        });
      } catch (e) {
        if (e instanceof SevenTVSessionExpired) {
          setConnected(false);
        } else {
          setLoadError(e instanceof Error ? e.message : String(e));
        }
      } finally {
        setLoading(false);
      }
    },
    [],
  );

  // Load on open; clear when closed so a reopen is always fresh.
  useEffect(() => {
    if (!showEmoteSetsOverlay) return;
    load(initialTwitchId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showEmoteSetsOverlay]);

  // Apply an initial tab once channels resolve (handled by ChannelWorkspace key).
  const startTab: WorkspaceTab = (initialTab as WorkspaceTab) || 'emotes';

  const connect = useCallback(async () => {
    setConnecting(true);
    try {
      await invoke('open_seventv_login_window');
    } catch (e) {
      addToast(e instanceof Error ? e.message : '7TV login failed to open', 'error');
      setConnecting(false);
    }
  }, [addToast]);

  // When the login window stores a token, reload.
  useEffect(() => {
    if (!showEmoteSetsOverlay) return;
    let un: (() => void) | undefined;
    let disposed = false;
    listen('seventv-connected', () => {
      setConnecting(false);
      load(initialTwitchId);
    }).then((u) => {
      // If we already unmounted before listen() resolved, unsubscribe now so the
      // handle can't leak.
      if (disposed) {
        u();
        return;
      }
      un = u;
    });
    return () => {
      disposed = true;
      un?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showEmoteSetsOverlay]);

  const onInvite = useCallback(
    async (channel: EditableChannel, accept: boolean) => {
      if (!mySeventvId) return;
      try {
        await respondToInvite(channel.seventvUserId, mySeventvId, accept);
        addToast(
          accept ? `Now editing ${channel.displayName}` : `Declined invite from ${channel.displayName}`,
          accept ? 'success' : 'info',
        );
        invalidateEditableChannels();
        await load();
        if (accept) setSelectedId(channel.seventvUserId);
      } catch (e) {
        if (e instanceof SevenTVSessionExpired) setConnected(false);
        addToast(e instanceof Error ? e.message : 'Could not respond to invite', 'error');
      }
    },
    [mySeventvId, addToast, load],
  );

  const selected = channels.find((c) => c.seventvUserId === selectedId) || null;
  const pendingInvites = channels.filter((c) => c.inviteState === 'PENDING');
  const usableChannels = channels.filter((c) => c.inviteState === 'ACCEPTED');

  return (
    <AnimatePresence>
      {showEmoteSetsOverlay && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.15 }}
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-2xl"
        >
          <div className="absolute inset-0" onClick={close} />

          <motion.div
            initial={{ opacity: 0, scale: 0.95, y: 15 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.95, y: 15 }}
            transition={{ type: 'spring', stiffness: 350, damping: 25 }}
            style={{ willChange: 'transform, opacity' }}
            className="w-[95vw] max-w-[1700px] h-[90vh] liquid-glass-panel flex flex-col overflow-hidden relative z-10"
          >
            {/* Header */}
            <div className="flex items-center justify-between px-4 py-3 border-b border-borderSubtle shrink-0">
              <h2 className="text-lg font-semibold text-textPrimary flex items-center gap-2.5">
                <SevenTVLogo className="h-5 w-auto text-[#29b6f6]" />
                7TV Emotes
              </h2>
              <div className="flex items-center gap-1">
                {connected && (
                  <Tooltip content="Refresh channels" delay={200} side="left">
                    <button
                      onClick={() => {
                        invalidateEditableChannels();
                        load();
                      }}
                      className="p-1.5 text-textSecondary hover:text-textPrimary hover:bg-glass rounded transition-all duration-200"
                    >
                      <RefreshCw size={16} />
                    </button>
                  </Tooltip>
                )}
                <Tooltip content="Close" delay={200} side="left">
                  <button
                    onClick={close}
                    className="p-1.5 text-textSecondary hover:text-textPrimary hover:bg-glass rounded transition-all duration-200"
                  >
                    <X size={18} />
                  </button>
                </Tooltip>
              </div>
            </div>

            {/* Body */}
            <div className="flex-1 min-h-0 flex">
              {loading ? (
                <div className="flex-1 flex items-center justify-center text-textSecondary">
                  <Loader2 size={22} className="animate-spin" />
                </div>
              ) : !connected ? (
                <ConnectPrompt onConnect={connect} connecting={connecting} />
              ) : loadError ? (
                <div className="flex-1 flex flex-col items-center justify-center gap-3 text-textSecondary px-6 text-center">
                  <ShieldAlert size={26} className="text-amber-400" />
                  <p className="max-w-md text-sm">{loadError}</p>
                  <button
                    onClick={() => load()}
                    className="glass-button px-3 py-1.5 text-sm text-textPrimary rounded"
                  >
                    Try again
                  </button>
                </div>
              ) : (
                <>
                  {/* Channel rail */}
                  <div className="w-64 shrink-0 border-r border-borderSubtle flex flex-col">
                    <div className="overflow-y-auto custom-scrollbar flex-1 p-2 space-y-1">
                      {pendingInvites.length > 0 && (
                        <div className="mb-2">
                          <div className="px-2 py-1 text-[11px] uppercase tracking-wide text-textMuted">
                            Pending invites
                          </div>
                          {pendingInvites.map((c) => (
                            <div
                              key={c.seventvUserId}
                              className="glass-panel rounded-lg p-2 mb-1.5 flex items-center gap-2"
                            >
                              <ChannelAvatar channel={c} />
                              <div className="min-w-0 flex-1">
                                <div className="text-sm text-textPrimary truncate">{c.displayName}</div>
                                <div className="text-[11px] text-textMuted">invited you to edit</div>
                              </div>
                              <Tooltip content="Accept" side="top" delay={150}>
                                <button
                                  onClick={() => onInvite(c, true)}
                                  className="p-1 rounded text-emerald-400 hover:bg-glass"
                                >
                                  <Check size={16} />
                                </button>
                              </Tooltip>
                              <Tooltip content="Decline" side="top" delay={150}>
                                <button
                                  onClick={() => onInvite(c, false)}
                                  className="p-1 rounded text-textSecondary hover:text-red-400 hover:bg-glass"
                                >
                                  <XCircle size={16} />
                                </button>
                              </Tooltip>
                            </div>
                          ))}
                        </div>
                      )}

                      {usableChannels.length === 0 && pendingInvites.length === 0 ? (
                        <div className="px-2 py-6 text-center text-xs text-textMuted">
                          You aren't a 7TV editor of any channel yet. Your own channel still appears here.
                        </div>
                      ) : (
                        usableChannels.map((c) => {
                          const active = c.seventvUserId === selectedId;
                          return (
                            <button
                              key={c.seventvUserId}
                              onClick={() => setSelectedId(c.seventvUserId)}
                              className={`w-full flex items-center gap-2 rounded-lg p-2 text-left transition-colors ${
                                active ? 'bg-glass-active' : 'hover:bg-glass'
                              }`}
                            >
                              <ChannelAvatar channel={c} />
                              <div className="min-w-0 flex-1">
                                <div className="text-sm text-textPrimary truncate flex items-center gap-1">
                                  {c.displayName}
                                  {c.isSelf && (
                                    <Tooltip content="Your channel" side="top" delay={150}>
                                      <Crown size={12} className="text-amber-400 shrink-0" />
                                    </Tooltip>
                                  )}
                                </div>
                                <div className="text-[11px] text-textMuted">
                                  {c.isSelf ? 'owner' : c.perms.manageEmotes ? 'editor' : 'view only'}
                                </div>
                              </div>
                            </button>
                          );
                        })
                      )}
                    </div>
                  </div>

                  {/* Workspace */}
                  <div className="flex-1 min-w-0">
                    {selected ? (
                      <ChannelWorkspace
                        key={selected.seventvUserId}
                        channel={selected}
                        initialTab={startTab}
                        onChannelsChanged={() => {
                          invalidateEditableChannels();
                          load();
                        }}
                      />
                    ) : (
                      <div className="h-full flex items-center justify-center text-sm text-textMuted">
                        Select a channel.
                      </div>
                    )}
                  </div>
                </>
              )}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

function ChannelAvatar({ channel }: { channel: EditableChannel }) {
  return channel.avatarUrl ? (
    <SevenTvAvatar
      cosmeticsId={channel.twitchId}
      px={28}
      src={channel.avatarUrl}
      alt=""
      className="w-7 h-7 rounded-full shrink-0 object-cover"
      loading="lazy"
    />
  ) : (
    <div className="w-7 h-7 rounded-full shrink-0 bg-glass flex items-center justify-center text-[11px] text-textSecondary">
      {channel.displayName.slice(0, 1).toUpperCase()}
    </div>
  );
}

function ConnectPrompt({ onConnect, connecting }: { onConnect: () => void; connecting: boolean }) {
  return (
    <div className="flex-1 flex flex-col items-center justify-center gap-4 px-6 text-center">
      <SevenTVLogo className="h-9 w-auto text-[#29b6f6]" />
      <div>
        <div className="text-textPrimary font-medium">Connect your 7TV account</div>
        <p className="text-sm text-textSecondary max-w-sm mt-1">
          Sign in with 7TV to manage emotes for your channel and any channel you're an editor of.
        </p>
      </div>
      <button
        onClick={onConnect}
        disabled={connecting}
        className="glass-button px-4 py-2 text-sm text-textPrimary rounded flex items-center gap-2 disabled:opacity-60"
      >
        {connecting && <Loader2 size={15} className="animate-spin" />}
        {connecting ? 'Waiting for 7TV...' : 'Connect 7TV'}
      </button>
    </div>
  );
}
