import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
  type MouseEvent as ReactMouseEvent,
} from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { motion, AnimatePresence } from 'framer-motion';
import { X, ExternalLink, Loader2, Copy, Pencil, Trash2, Clapperboard, Check, ArrowUpRight, MessageSquare } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import type { ClipSource, MediaInfo } from '../stores/AppStore';
import { Logger } from '../utils/logger';
import ChatMessageList from './ChatMessageList';
import { openProfilePopup } from '../utils/openProfilePopup';
import { useChatCosmetics } from '../hooks/useChatCosmetics';
import {
  beginVodReplay,
  clipReplayWindow,
  restoreReplaySession,
  snapshotReplaySession,
  useVodReplaySnapshot,
  useVodReplayStore,
  type ReplaySession,
} from '../stores/vodReplayStore';

interface ClipResolveResult {
  url: string;
  quality: string;
  available: string[];
  /** Where this clip sits in its source broadcast, for chat replay. */
  clip_source?: ClipSource;
}

const NOOP = () => {};
const EMPTY_IDS: Set<string> = new Set();
const EMPTY_CONTEXTS = new Map<string, never>();
// Replay comments arrive already tokenized into segments by the Rust parser, so the
// list never has to resolve an emote set itself.
const getReplayMessageId = (m: string | { id?: string }): string | null =>
  typeof m === 'string' ? null : (m.id ?? null);

/** The chat that was live while this clip was being recorded.
 *
 *  The clip plays its own MP4 as always; this only reads the source broadcast's
 *  comments, because Twitch addresses them by video id + offset and has no
 *  clip-addressed equivalent. */
function ClipChatPanel({
  hasSource,
  channelName,
}: {
  hasSource: boolean;
  /** Broadcaster of the source VOD. Threaded through so a profile opened from
   *  clip chat resolves badges and message history against the right channel
   *  instead of falling back to the clicked user's own channel. */
  channelName?: string;
}) {
  const chat = useVodReplaySnapshot();
  const error = useVodReplayStore((s) => s.error);
  const active = useVodReplayStore((s) => s.active);
  // This panel renders ChatMessageList directly instead of going through ChatWidget, so
  // it has to resolve account-level cosmetics itself (paints, 7TV + member badges,
  // atmospheres). Without it the rows render, just stripped of everything that isn't
  // carried on the message.
  useChatCosmetics(chat.messages);

  // Every branch is `absolute inset-0`, which is load-bearing: the column is a flex item
  // stretched to the row's height, and if this content were in flow its intrinsic height
  // (hundreds of message rows) would define the row instead, stretching the video box to
  // thousands of pixels and destroying its 16:9 box. Taking it out of flow makes the
  // video the only thing that sizes the row, and gives the list a definite height to
  // scroll inside.
  if (!hasSource) {
    return (
      <div className="absolute inset-0 flex items-center justify-center px-6 text-center">
        <p className="text-xs leading-relaxed text-textSecondary">
          No chat for this clip.
          <br />
          <span className="text-textMuted">Its broadcast is no longer available.</span>
        </p>
      </div>
    );
  }
  if (error) {
    return (
      <div className="absolute inset-0 flex items-center justify-center px-6 text-center">
        <p className="text-xs text-textSecondary">Couldn&apos;t load the chat for this clip.</p>
      </div>
    );
  }
  return (
    <div className="absolute inset-0">
      <ChatMessageList
        messages={chat.messages}
        renderToken={chat.renderToken}
        isPaused={false}
        onScroll={NOOP}
        onUsernameClick={(userId, username, displayName, color, badges, event) => {
          void openProfilePopup({
            userId,
            username,
            displayName,
            color,
            badges,
            channelName,
            clientX: event.clientX,
            clientY: event.clientY,
          });
        }}
        onReplyClick={NOOP}
        onEmoteRightClick={NOOP}
        onUsernameRightClick={NOOP}
        onBadgeClick={NOOP}
        highlightedMessageId={null}
        deletedMessageIds={EMPTY_IDS}
        clearedUserContexts={EMPTY_CONTEXTS}
        emotes={null}
        getMessageId={getReplayMessageId}
      />
      {!active && (
        <div className="pointer-events-none absolute inset-x-0 bottom-2 text-center text-[11px] text-textMuted">
          Loading chat…
        </div>
      )}
    </div>
  );
}

function FooterBtn({
  icon,
  label,
  onClick,
  danger,
  primary,
  className,
}: {
  icon: ReactNode;
  label: string;
  onClick: (e: ReactMouseEvent) => void;
  danger?: boolean;
  primary?: boolean;
  className?: string;
}) {
  // Built on the app's `glass-button` primitive (inset bevel + depth). Primary
  // tints the surface Twitch-purple; danger/secondary just recolor the label.
  // Only the emphasized action (`primary`) is a real glass button; the rest are
  // flat icon+label actions (no bevel, just a subtle hover) so the main action
  // stands out.
  const variant = primary
    ? 'glass-button text-[#bf94ff] hover:text-[#d4b3ff]'
    : danger
      ? 'text-red-400 hover:bg-red-500/[0.1] hover:text-red-300'
      : 'text-textSecondary hover:bg-white/[0.05] hover:text-textPrimary';
  return (
    <button
      type="button"
      onClick={onClick}
      className={`inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-xs font-semibold transition-all ${variant} ${className ?? ''}`}
    >
      {icon}
      {label}
    </button>
  );
}

// A purple "drop" that flies from a point to a target — used to show a clip
// being sent to chat (mirrors the MultiNook add-from-home drop). Self-portals to
// <body> so it animates above the modal. Removed by the parent after it lands.
function PurpleDrop({
  startX,
  startY,
  targetX,
  targetY,
}: {
  startX: number;
  startY: number;
  targetX: number;
  targetY: number;
}) {
  const [flying, setFlying] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setFlying(true), 10);
    return () => clearTimeout(t);
  }, []);
  return createPortal(
    <div
      className="pointer-events-none fixed z-[9999] flex h-5 w-5 items-center justify-center rounded-full"
      style={{
        left: flying ? targetX : startX,
        top: flying ? targetY : startY,
        opacity: flying ? 0.15 : 1,
        transform: `translate(-50%, -50%) scale(${flying ? 0.3 : 1})`,
        transition: 'all 600ms cubic-bezier(0.22, 1, 0.36, 1)',
        backgroundColor: '#9146FF',
        boxShadow: '0 0 14px rgba(145,70,255,0.35)',
      }}
    >
      <ArrowUpRight size={12} className="text-white" />
    </div>,
    document.body,
  );
}

// A centered overlay player for a Twitch clip posted in chat. A clip is a direct
// signed MP4 (no streaming server), so it plays in its own <video> here while the
// main stream/chat stays mounted underneath — closing the modal lands the viewer
// back exactly where they were. Rendered in both the main window and MultiChat
// popouts, so a clip opens in whichever window it was clicked (no window jump).
export default function ClipModal() {
  const clipModal = useAppStore((s) => s.clipModal);
  const close = useAppStore((s) => s.closeClipModal);
  const quality = useAppStore((s) => s.settings?.quality) ?? 'best';

  return (
    <AnimatePresence>
      {clipModal && (
        <ClipModalInner
          key={clipModal.url}
          url={clipModal.url}
          info={clipModal.info}
          created={clipModal.created}
          editUrl={clipModal.editUrl}
          shareOnly={clipModal.shareOnly}
          quality={quality}
          onClose={close}
        />
      )}
    </AnimatePresence>
  );
}

function ClipModalInner({
  url,
  info,
  quality,
  onClose,
  created,
  editUrl,
  shareOnly,
}: {
  url: string;
  info: MediaInfo;
  quality: string;
  onClose: () => void;
  created?: boolean;
  editUrl?: string;
  shareOnly?: boolean;
}) {
  const [src, setSrc] = useState<string | null>(null);
  const [error, setError] = useState(false);
  const videoRef = useRef<HTMLVideoElement | null>(null);
  // The resolver looks these up for any clip, so it is the more reliable source; the
  // caller's copy covers a clip opened straight from a grid before the resolve lands.
  const [resolvedSource, setResolvedSource] = useState<ClipSource | null>(null);
  const clipSource = resolvedSource ?? info.clip_source ?? null;
  const hasClipSource = !!clipSource?.video_id && clipSource?.vod_offset != null;
  const showChat = useAppStore((s) => s.settings.clip_chat_replay !== false);
  const setShowChat = useAppStore((s) => s.updateSettings);
  const settings = useAppStore((s) => s.settings);
  const addToast = useAppStore((s) => s.addToast);
  const channelLogin = useAppStore((s) => s.currentStream?.user_login);
  const [pendingDelete, setPendingDelete] = useState(false);
  // Purple "sent to chat" drops in flight (each flies from the click → chat input).
  const [drops, setDrops] = useState<
    Array<{ id: number; startX: number; startY: number; targetX: number; targetY: number }>
  >([]);
  const dropIdRef = useRef(0);
  // The clip slug (handles clips.twitch.tv/<slug> and twitch.tv/<ch>/clip/<slug>).
  const slug =
    url.match(/clips\.twitch\.tv\/([^/?#]+)/)?.[1] ||
    url.match(/\/clip\/([^/?#]+)/)?.[1] ||
    info.id ||
    '';

  // Release the media element when it detaches (modal close / clip change). A
  // detached <video> can keep decoding, buffering, and even playing audio until
  // GC reclaims it; pausing + clearing the src and calling load() frees it
  // immediately. Done via a ref callback (React calls it with null on detach) so
  // we always release the exact node that's going away.
  const attachVideo = useCallback((el: HTMLVideoElement | null) => {
    if (el === null && videoRef.current) {
      const prev = videoRef.current;
      prev.pause();
      prev.removeAttribute('src');
      prev.load();
    }
    videoRef.current = el;
  }, []);

  // The chat that was live while this clip was recorded. The clip still plays its own
  // MP4; the source broadcast is only the address for the comments.
  //
  // The replay engine is a module singleton, so if a VOD replay was already running when
  // this opened (a clip link clicked mid-VOD is completely ordinary), snapshot it and put
  // it back on close. Without that, closing the modal leaves the VOD with dead chat.
  useEffect(() => {
    const videoId = clipSource?.video_id;
    if (!showChat || !hasClipSource || !src || !videoId) return;
    const previous: ReplaySession | null = snapshotReplaySession();
    beginVodReplay(videoId, clipSource?.broadcaster_login || channelLogin || '', {
      ...clipReplayWindow(clipSource?.vod_offset ?? 0, clipSource?.duration),
      getTime: () => videoRef.current?.currentTime ?? null,
    });
    return () => restoreReplaySession(previous);
  }, [
    showChat,
    hasClipSource,
    src,
    channelLogin,
    clipSource?.video_id,
    clipSource?.vod_offset,
    clipSource?.duration,
    clipSource?.broadcaster_login,
  ]);

  // Get the playable source. A freshly-created clip (`created`) needs Twitch to
  // finish RENDERING it first — resolving too early yields a black frame that
  // never refreshes — so poll ShareClipRenderStatus until CREATED, then resolve
  // the SIGNED playback URL. An already-finalized clip (a chat link) resolves
  // directly. Cancellation guards against a result landing after close.
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let attempts = 0;
    setSrc(null);
    setError(false);

    const fail = (e: unknown) => {
      if (cancelled) return;
      Logger.error('[ClipModal] could not load clip:', e);
      setError(true);
    };

    // resolve_clip_media returns a SIGNED, playable URL. (The raw `sourceURL`
    // from ShareClipRenderStatus is unsigned → 401, so that op is used only to
    // learn readiness.) A little retry covers transient hiccups.
    const resolveSrc = (retriesLeft: number) => {
      invoke<ClipResolveResult>('resolve_clip_media', { url, quality })
        .then((r) => {
          if (cancelled) return;
          setSrc(r.url);
          if (r.clip_source) setResolvedSource(r.clip_source);
        })
        .catch((e) => {
          if (cancelled) return;
          if (retriesLeft > 0) {
            timer = setTimeout(() => resolveSrc(retriesLeft - 1), 1500);
          } else {
            fail(e);
          }
        });
    };

    // Freshly-created clip: wait until Twitch finishes RENDERING it (else the
    // asset is a black 0:00 frame), THEN resolve the signed URL to play.
    const waitThenResolve = () => {
      invoke<{ ready: boolean }>('get_clip_render_status', { slug })
        .then((st) => {
          if (cancelled) return;
          if (st.ready) {
            resolveSrc(3);
            return;
          }
          attempts += 1;
          if (attempts > 30) return fail('clip render timed out'); // ~60s ceiling
          timer = setTimeout(waitThenResolve, 2000);
        })
        .catch((e) => {
          if (cancelled) return;
          attempts += 1;
          if (attempts > 30) return fail(e);
          timer = setTimeout(waitThenResolve, 2000);
        });
    };

    // Share-only: no player, so don't fetch footage at all.
    if (!shareOnly) {
      if (created) waitThenResolve();
      else resolveSrc(4);
    }

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [url, quality, created, slug, shareOnly]);

  // Esc closes — the keyboard counterpart to the backdrop click and the X.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const openExternal = async (target: string = url) => {
    try {
      const { open } = await import('@tauri-apps/plugin-shell');
      await open(target);
    } catch (err) {
      Logger.error('[ClipModal] external open failed:', err);
    }
  };

  const copyLink = async () => {
    try {
      await navigator.clipboard.writeText(url);
      addToast('Clip link copied', 'success');
    } catch {
      addToast('Could not copy link', 'error');
    }
  };

  const sendToChat = (e: ReactMouseEvent) => {
    const { currentStream, currentUser } = useAppStore.getState();
    if (!currentStream?.user_login) {
      addToast('No channel to post to', 'warning');
      return;
    }
    // Fly a purple drop from the click toward the chat input box — a visual cue
    // that the clip just went to chat (mirrors the MultiNook add animation).
    const input = document.getElementById('chat-compose-input');
    if (input) {
      const r = input.getBoundingClientRect();
      const id = (dropIdRef.current += 1);
      setDrops((prev) => [
        ...prev,
        {
          id,
          startX: e.clientX,
          startY: e.clientY,
          targetX: r.left + r.width / 2,
          targetY: r.top + r.height / 2,
        },
      ]);
      window.setTimeout(() => setDrops((prev) => prev.filter((d) => d.id !== id)), 700);
    }
    void (async () => {
      try {
        await invoke('send_chat_message', {
          message: url,
          replyParentMsgId: null,
          targetChannel: currentStream.user_login,
          broadcasterId: currentStream.user_id || null,
          senderId: currentUser?.user_id || null,
          senderAccountId: null,
        });
        addToast('Clip posted to chat', 'success');
      } catch (err) {
        Logger.error('[ClipModal] send to chat failed:', err);
        addToast('Could not post to chat', 'error');
      }
    })();
  };

  // Delete is two-step (first click arms it, second confirms) so a freshly-made
  // clip isn't nuked by a stray click. Only offered for clips you own (created).
  const deleteClip = async () => {
    if (!pendingDelete) {
      setPendingDelete(true);
      setTimeout(() => setPendingDelete(false), 3000);
      return;
    }
    try {
      await invoke('delete_clip', { slug });
      addToast('Clip deleted', 'success');
      onClose();
    } catch (err) {
      Logger.error('[ClipModal] delete failed:', err);
      addToast('Could not delete clip', 'error');
    }
  };

  const channel = info.broadcaster_name || info.user_name;

  const actionsFooter = (
    <div className="flex flex-wrap items-center gap-1.5 px-3 py-2.5">
      {channelLogin && (
        <FooterBtn primary icon={<ArrowUpRight size={15} />} label="Send to chat" onClick={sendToChat} />
      )}
      <FooterBtn icon={<Copy size={15} />} label="Copy link" onClick={copyLink} />
      {editUrl && (
        <FooterBtn icon={<Pencil size={15} />} label="Edit on Twitch" onClick={() => openExternal(editUrl)} />
      )}
      <FooterBtn icon={<ExternalLink size={15} />} label="Open in browser" onClick={() => openExternal(url)} />
      {created && (
        <FooterBtn
          icon={<Trash2 size={15} />}
          label={pendingDelete ? 'Confirm delete?' : 'Delete'}
          onClick={deleteClip}
          danger
          className="ml-auto"
        />
      )}
    </div>
  );

  const flyingDrops = drops.map((d) => (
    <PurpleDrop key={d.id} startX={d.startX} startY={d.startY} targetX={d.targetX} targetY={d.targetY} />
  ));

  // Share-only: the clip was just made + previewed in the editor, so skip the
  // player and show a clean "share your clip" card — just the actions.
  if (shareOnly) {
    return createPortal(
      <motion.div
        className="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 p-6 backdrop-blur-sm"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.15 }}
        onClick={onClose}
      >
        <motion.div
          className="glass-panel relative w-full max-w-md overflow-hidden"
          initial={{ scale: 0.96, opacity: 0 }}
          animate={{ scale: 1, opacity: 1 }}
          exit={{ scale: 0.96, opacity: 0 }}
          transition={{ duration: 0.15, ease: 'easeOut' }}
          onClick={(e) => e.stopPropagation()}
        >
          <div className="flex items-start gap-2.5 px-4 pb-3 pt-4">
            <Clapperboard size={20} style={{ color: '#9146FF' }} className="mt-0.5 flex-shrink-0" />
            <div className="min-w-0 flex-1">
              <div className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: '#bf94ff' }}>
                Clip created
              </div>
              {info.title && (
                <div className="truncate text-base font-semibold leading-snug text-textPrimary">
                  {info.title}
                </div>
              )}
              <div className="truncate text-xs text-textSecondary">
                <span style={{ color: '#9146FF' }} className="font-semibold">
                  Twitch
                </span>
                {channel ? ` · ${channel}` : ''}
              </div>
            </div>
          </div>
          <div className="mx-3 border-t border-white/[0.06]" />
          <div className="space-y-1.5 px-3 pb-3 pt-2.5">
            <div className="grid grid-cols-2 gap-1">
              <FooterBtn className="w-full" icon={<Copy size={15} />} label="Copy link" onClick={copyLink} />
              {channelLogin && (
                <FooterBtn
                  primary
                  className="w-full"
                  icon={<ArrowUpRight size={15} />}
                  label="Send to chat"
                  onClick={sendToChat}
                />
              )}
            </div>
            {flyingDrops}
            <div className="flex items-center justify-between pt-1">
              {created ? (
                <FooterBtn
                  danger
                  icon={<Trash2 size={15} />}
                  label={pendingDelete ? 'Confirm delete?' : 'Delete'}
                  onClick={deleteClip}
                />
              ) : (
                <span />
              )}
              <button
                type="button"
                onClick={onClose}
                className="glass-button inline-flex items-center gap-1.5 rounded-lg px-4 py-2 text-xs font-semibold text-textPrimary transition-all"
              >
                <Check size={15} /> Done
              </button>
            </div>
          </div>
        </motion.div>
      </motion.div>,
      document.body,
    );
  }

  return createPortal(
    <motion.div
      className="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 p-6 backdrop-blur-sm"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.15 }}
      onClick={onClose}
    >
      <motion.div
        className="glass-panel relative w-full max-w-7xl overflow-hidden"
        initial={{ scale: 0.96, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        exit={{ scale: 0.96, opacity: 0 }}
        transition={{ duration: 0.15, ease: 'easeOut' }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 px-4 py-2.5">
          <div className="min-w-0 flex-1">
            {info.title && (
              <div className="truncate text-sm font-medium text-textPrimary">{info.title}</div>
            )}
            <div className="truncate text-xs text-textSecondary">
              <span style={{ color: '#9146FF' }} className="font-semibold">
                Twitch
              </span>
              {channel ? ` · ${channel}` : ''}
            </div>
          </div>
          <button
            type="button"
            onClick={() => setShowChat({ ...settings, clip_chat_replay: !showChat })}
            aria-label={showChat ? 'Hide chat replay' : 'Show chat replay'}
            title={showChat ? 'Hide chat replay' : 'Show chat replay'}
            className={`hidden h-8 w-8 flex-shrink-0 items-center justify-center rounded-md transition-colors hover:bg-white/[0.06] lg:flex ${
              showChat ? 'text-textPrimary' : 'text-textSecondary hover:text-textPrimary'
            }`}
          >
            <MessageSquare size={16} />
          </button>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close clip"
            className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-md text-textSecondary transition-colors hover:bg-white/[0.06] hover:text-textPrimary"
          >
            <X size={18} />
          </button>
        </div>

        <div className="flex w-full flex-col lg:flex-row">
        <div className="relative w-full min-w-0 flex-1 bg-black" style={{ aspectRatio: '16 / 9' }}>
          {src ? (
            <video
              ref={attachVideo}
              src={src}
              autoPlay
              controls
              className="h-full w-full"
              onError={() => setError(true)}
            />
          ) : error ? (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 px-6 text-center">
              <p className="text-sm text-textSecondary">Couldn&apos;t load this clip.</p>
              <button
                type="button"
                onClick={() => openExternal()}
                className="inline-flex items-center gap-1.5 rounded-md bg-white/[0.06] px-3 py-1.5 text-xs text-textPrimary transition-colors hover:bg-white/[0.1]"
              >
                <ExternalLink size={13} /> Open in browser
              </button>
            </div>
          ) : (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-2">
              <Loader2 size={28} className="animate-spin text-textSecondary" />
              {created && <p className="text-xs text-textSecondary">Preparing your clip…</p>}
            </div>
          )}
        </div>
        {/* Hidden under lg so a narrow window keeps the full-width video rather than
            squeezing both. */}
        {showChat && (
          <div className="relative hidden w-[340px] flex-shrink-0 self-stretch border-l border-borderSubtle lg:block">
            <ClipChatPanel
              hasSource={hasClipSource}
              channelName={clipSource?.broadcaster_login || channelLogin || undefined}
            />
          </div>
        )}
        </div>

        {/* Action bar — the all-in-one: watch above, act below. */}
        {actionsFooter}
        {flyingDrops}
      </motion.div>
    </motion.div>,
    document.body,
  );
}
