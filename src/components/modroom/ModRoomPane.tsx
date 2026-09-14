import { useEffect, useRef, useState, useCallback, useMemo, Fragment, useSyncExternalStore } from 'react';
import type { ReactNode, MouseEvent, CSSProperties, ChangeEvent, ClipboardEvent, KeyboardEvent } from 'react';
import { ShieldCheck, Paperclip, X, CornerUpLeft, Pencil } from 'lucide-react';
import { EmotePickerPanel, useSwappingSmiley } from '../chat/EmotePickerPanel';
import { open as openExternal } from '@tauri-apps/plugin-shell';
import { connectModRoomConsent, type ModRoomChat, type ModRoomMember } from '../../services/modRoomService';
import { isEncrypted, encryptText, encryptBytes, decryptBytes } from '../../services/modRoomCrypto';
import {
  ensureRoom,
  releaseRoom,
  retryRoom,
  setViewing,
  sendOptimistic,
  sendEdit,
  sendTyping,
  uploadAttachment,
  retryPending,
  discardPending,
  mentionsMe,
} from '../../services/modRoomManager';
import { useModRoomStore, type ResolvedPayload } from '../../stores/modRoomStore';
import { StreamNookBadge } from '../StreamNookBadge';
import { Tooltip } from '../ui/Tooltip';
import SpellcheckUnderlay from '../chat/SpellcheckUnderlay';
import { useSpellcheck } from '../../hooks/useSpellcheck';
import { warmSpellcheck } from '../../utils/spellcheck';
import { AtmosphereBackground } from '../AtmosphereBackground';
import { MajorCologneChrome } from '../MajorCologneChrome';
import { MAJOR_COLOGNE_THEME_ID } from '../../services/cologneEvent';
import { computePaintStyle } from '../../services/seventvService';
import { getAtmosphere } from '../../services/atmospheres';
import {
  isStreamNookUser,
  getStreamNookUserNumber,
  subscribeCosmeticsVersion,
  getCosmeticsVersion,
  subscribeStreamNookRegistryVersion,
  getStreamNookRegistryVersion,
} from '../../services/supabaseService';
import { useChatUserStore, ensureAtmosphereResolved } from '../../stores/chatUserStore';
import { useAppStore } from '../../stores/AppStore';
import { parseEmojisSync, getAppleEmojiUrl } from '../../services/emojiService';
import type { EmoteSet, Emote } from '../../services/emoteService';

interface ModRoomPaneProps {
  channelId: string;
  channelLogin?: string;
  emotes?: EmoteSet | null;
  /** Reports room state up so the host (chat header) can display it. */
  onStatus?: (s: { memberCount: number; encrypted: boolean; connected: boolean; members: ModRoomMember[] }) => void;
  onUsernameClick?: (login: string, userId: string, event: MouseEvent) => void;
}

const TYPING_TTL_MS = 3000;
const TYPING_PING_MS = 1500;
// Plaintext bound. The server caps the CIPHERTEXT at 16000 chars and rejects
// (never truncates) oversize bodies; these caps keep the worst-case encrypted
// payload comfortably under that.
const MAX_DRAFT = 4000;
const MAX_EMOTE_REFS = 40;
const MAX_TOKEN_CHARS = 15000;

interface ReplyRef {
  id: string;
  login: string;
  text: string;
}

// An emote the sender used, embedded in the message so it renders identically for
// everyone, forever — independent of the viewer's emote set or future changes.
interface EmoteRef {
  n: string; // name (the token in the text)
  id: string;
  p: string; // provider
  u: string; // url
}

type LockState = false | 'pending' | 'failed';

// Stable fallbacks so hooks keyed on these don't churn while a session boots.
const EMPTY_MESSAGES: ModRoomChat[] = [];
const EMPTY_MEMBERS: ModRoomMember[] = [];

function roleColorClass(role: string): string {
  if (role === 'broadcaster') return 'text-[#f0c674]';
  if (role === 'moderator') return 'text-accent';
  return 'text-textSecondary';
}

// Register a sender with the shared chat user store so its cosmetics (7TV paint,
// atmosphere, StreamNook badge) resolve once and decorate the row, like live chat.
// addUser only resolves the atmosphere on first sight, so prod it explicitly for
// users already known from live chat (the fast path skips it).
function ensureUser(userId: string, login: string) {
  if (!userId) return;
  useChatUserStore.getState().addUser({ userId, username: login, displayName: login, color: '' });
  ensureAtmosphereResolved(userId);
}

// ----- emote/emoji tokenizer ------------------------------------------------

type BodySeg =
  | { kind: 'text'; text: string }
  | { kind: 'emote'; name: string; url: string }
  | { kind: 'emoji'; alt: string; url: string }
  | { kind: 'link'; url: string };

function findEmote(word: string, emotes?: EmoteSet | null): Emote | undefined {
  if (!emotes) return undefined;
  return (
    emotes['7tv']?.find((e) => e.name === word) ||
    emotes.bttv?.find((e) => e.name === word) ||
    emotes.ffz?.find((e) => e.name === word) ||
    emotes.twitch?.find((e) => e.name === word)
  );
}

// Collect the emotes a message used, resolved once at send time against the
// sender's full emote set, so they travel WITH the message (persistent +
// consistent for every viewer). Capped so the encrypted payload stays bounded.
function collectEmoteRefs(text: string, emotes?: EmoteSet | null): EmoteRef[] {
  if (!emotes) return [];
  const out: EmoteRef[] = [];
  const seen = new Set<string>();
  for (const word of text.split(/\s+/)) {
    if (!word || seen.has(word)) continue;
    const e = findEmote(word, emotes);
    if (e) {
      seen.add(word);
      out.push({ n: e.name, id: e.id, p: e.provider, u: e.url });
      if (out.length >= MAX_EMOTE_REFS) break;
    }
  }
  return out;
}

function tokenizeBody(body: string, emotes?: EmoteSet | null, embedded?: Map<string, EmoteRef>): BodySeg[] {
  const segs: BodySeg[] = [];
  for (const part of body.split(/(\s+)/)) {
    if (!part) continue;
    if (/^\s+$/.test(part)) {
      segs.push({ kind: 'text', text: part });
      continue;
    }
    // Embedded refs win (exact image the sender used); fall back to the viewer's
    // channel set only for legacy messages that carry no embedded emotes.
    const ref = embedded?.get(part);
    if (ref) {
      segs.push({ kind: 'emote', name: ref.n, url: ref.u });
      continue;
    }
    const emote = findEmote(part, emotes);
    if (emote) {
      segs.push({ kind: 'emote', name: emote.name, url: emote.url });
      continue;
    }
    if (/^https?:\/\/\S+$/i.test(part)) {
      segs.push({ kind: 'link', url: part });
      continue;
    }
    for (const es of parseEmojisSync(part)) {
      if (es.type === 'emoji' && es.emojiUrl) segs.push({ kind: 'emoji', alt: es.content, url: es.emojiUrl });
      else segs.push({ kind: 'text', text: es.content });
    }
  }
  return segs;
}

function renderSeg(seg: BodySeg, i: number): ReactNode {
  if (seg.kind === 'emote') {
    return (
      <Tooltip key={i} content={seg.name}>
        <img
          src={seg.url}
          alt={seg.name}
          loading="lazy"
          className="mx-px inline-block align-middle"
          style={{ height: '1.8em', maxWidth: '9em', objectFit: 'contain' }}
        />
      </Tooltip>
    );
  }
  if (seg.kind === 'emoji') {
    return (
      <img
        key={i}
        src={seg.url}
        alt={seg.alt}
        loading="lazy"
        className="mx-px inline-block align-middle"
        style={{ height: '1.3em', width: '1.3em' }}
      />
    );
  }
  if (seg.kind === 'link') {
    return (
      <button
        key={i}
        onClick={() => void openExternal(seg.url)}
        className="break-all align-middle text-accent underline decoration-accent/40 underline-offset-2 transition-colors hover:decoration-accent"
      >
        {seg.url}
      </button>
    );
  }
  return <span key={i}>{seg.text}</span>;
}

function dayLabel(ts: number): string {
  const d = new Date(ts);
  const today = new Date();
  if (d.toDateString() === today.toDateString()) return 'Today';
  const yesterday = new Date(today.getTime() - 86_400_000);
  if (d.toDateString() === yesterday.toDateString()) return 'Yesterday';
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

const HairlineDivider = ({ label, accent }: { label: string; accent?: boolean }) => (
  <div className="my-2 flex items-center justify-center gap-2 px-6">
    <span className={`h-px w-10 ${accent ? 'bg-accent/40' : 'bg-white/10'}`} />
    <span className={`text-[10px] font-medium ${accent ? 'text-accent' : 'text-textSecondary'}`}>{label}</span>
    <span className={`h-px w-10 ${accent ? 'bg-accent/40' : 'bg-white/10'}`} />
  </div>
);

// ----- one message row, decorated like live chat ----------------------------

const ModRoomMessageRow = ({
  m,
  body,
  attachment,
  reply,
  emoteRefs,
  locked,
  canEdit,
  emotes,
  fontSize,
  roomKey,
  pending,
  sendFailed,
  mentioned,
  onUsernameClick,
  onReply,
  onEdit,
  onRetry,
  onDiscard,
}: {
  m: ModRoomChat;
  body: string;
  attachment?: string;
  reply?: ReplyRef;
  emoteRefs?: EmoteRef[];
  locked: LockState;
  canEdit: boolean;
  emotes?: EmoteSet | null;
  fontSize: number;
  roomKey: CryptoKey | null;
  pending?: boolean;
  sendFailed?: boolean;
  mentioned?: boolean;
  onUsernameClick?: (login: string, userId: string, event: MouseEvent) => void;
  onReply: () => void;
  onEdit: () => void;
  onRetry?: () => void;
  onDiscard?: () => void;
}) => {
  useSyncExternalStore(subscribeCosmeticsVersion, getCosmeticsVersion);
  // The StreamNook registry (badge / member status) and theme catalog load async;
  // re-render when they do, the same way ChatMessage does.
  useSyncExternalStore(subscribeStreamNookRegistryVersion, getStreamNookRegistryVersion, getStreamNookRegistryVersion);
  const paint = useChatUserStore((s) => s.users.get(m.userId)?.paint);
  const storeColor = useChatUserStore((s) => s.users.get(m.userId)?.color);
  const atmosphereId = useChatUserStore((s) => s.users.get(m.userId)?.atmosphereId ?? null);
  const cologne = useChatUserStore((s) => s.users.get(m.userId)?.cologne ?? null);
  const paintShadowMode = useAppStore((s) => s.settings.cosmetics?.paint_shadows) ?? 'all';

  const isSN = isStreamNookUser(m.userId);
  const userNumber = getStreamNookUserNumber(m.userId);
  const atmosphere = getAtmosphere(atmosphereId);
  const cologneAtm = cologne ? getAtmosphere(MAJOR_COLOGNE_THEME_ID) : null;
  const embedded = useMemo(
    () => (emoteRefs && emoteRefs.length ? new Map(emoteRefs.map((r) => [r.n, r])) : undefined),
    [emoteRefs],
  );
  const segments = useMemo(() => (locked ? [] : tokenizeBody(body, emotes, embedded)), [body, locked, emotes, embedded]);
  const time = useMemo(() => new Date(m.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }), [m.ts]);
  const nameStyle: CSSProperties | undefined = paint
    ? computePaintStyle(paint, storeColor || undefined, paintShadowMode)
    : undefined;

  const rowStyle = {
    fontSize: `${fontSize}px`,
    fontWeight: 'var(--chat-body-weight, 300)',
    // The Cologne frame needs horizontal room + a min height so the gold border
    // is never clipped on a single line (mirrors ChatMessage).
    ...(cologne?.frame ? { paddingLeft: 18, paddingRight: 18, paddingTop: 7, paddingBottom: 7, minHeight: 36 } : {}),
  } as CSSProperties;

  // `isolate` creates a stacking context so the -z-10 atmosphere/cologne wash
  // paints behind this row's content but ABOVE the chat panel background. Without
  // it the wash sinks behind the opaque panel and never shows.
  return (
    <div
      className={`group relative isolate px-1 py-0.5 leading-snug hover:bg-glass ${mentioned ? 'bg-accent/10' : ''}`}
      style={rowStyle}
    >
      {cologne && cologneAtm ? (
        <MajorCologneChrome
          textureUrl={cologneAtm.chromeTexture ?? ''}
          coinUrl={cologneAtm.chromeCoin}
          frameUrl={cologneAtm.chromeFrame}
          coin={cologne.coin}
          frame={cologne.frame}
        />
      ) : atmosphere ? (
        <AtmosphereBackground atm={atmosphere} variant="chat" />
      ) : null}
      <div className={`relative z-10 ${pending && !sendFailed ? 'opacity-60' : ''}`}>
        {reply && (
          <div className="mb-0.5 flex items-center gap-1 pl-1 text-[11px] text-textSecondary">
            <CornerUpLeft size={11} className="shrink-0" />
            <span className="shrink-0 font-semibold">{reply.login}</span>
            <span className="truncate opacity-80">{reply.text}</span>
          </div>
        )}
        <span>
          <span className="mr-1.5 align-middle text-[10px] tabular-nums text-textSecondary">{time}</span>
          {isSN && (
            <span className="mr-1 inline-flex align-middle">
              <StreamNookBadge userId={m.userId} userNumber={userNumber} />
            </span>
          )}
          {onUsernameClick ? (
            <button
              onClick={(e) => onUsernameClick(m.login, m.userId, e)}
              style={nameStyle}
              className={`mr-1.5 align-middle font-semibold hover:underline ${nameStyle ? '' : roleColorClass(m.role)}`}
            >
              {m.login}
            </button>
          ) : (
            <span style={nameStyle} className={`mr-1.5 align-middle font-semibold ${nameStyle ? '' : roleColorClass(m.role)}`}>
              {m.login}
            </span>
          )}
          {locked ? (
            <span className="align-middle italic text-textSecondary">
              {locked === 'failed' ? 'Unable to decrypt' : 'decrypting...'}
            </span>
          ) : (
            <span className="align-middle text-textPrimary break-words">
              {segments.map(renderSeg)}
              {m.editedAt ? (
                <span className="ml-1 align-middle text-[10px] text-textSecondary">(edited)</span>
              ) : null}
              {sendFailed && (
                <span className="ml-1.5 inline-flex items-center gap-1.5 align-middle text-[10px]">
                  <span className="font-semibold text-red-400">Failed</span>
                  <button onClick={onRetry} className="text-textSecondary underline underline-offset-2 transition-colors hover:text-textPrimary">
                    Retry
                  </button>
                  <button onClick={onDiscard} className="text-textSecondary underline underline-offset-2 transition-colors hover:text-textPrimary">
                    Discard
                  </button>
                </span>
              )}
            </span>
          )}
        </span>
        {!locked && attachment && (
          <div className="mt-1">
            <AttachmentImage url={attachment} roomKey={roomKey} />
          </div>
        )}
      </div>
      {!locked && !pending && (
        <div
          className="absolute right-1 top-0 z-20 hidden items-center gap-0.5 rounded-md group-hover:flex"
          style={{ background: 'rgba(20,20,22,0.92)', boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.06)' }}
        >
          <button
            onClick={onReply}
            aria-label="Reply"
            className="grid h-6 w-6 place-items-center rounded text-textSecondary transition-colors hover:text-accent"
          >
            <CornerUpLeft size={13} />
          </button>
          {canEdit && (
            <button
              onClick={onEdit}
              aria-label="Edit"
              className="grid h-6 w-6 place-items-center rounded text-textSecondary transition-colors hover:text-accent"
            >
              <Pencil size={13} />
            </button>
          )}
        </div>
      )}
    </div>
  );
};

// ----- the pane -------------------------------------------------------------

const ModRoomPane = ({ channelId, channelLogin, emotes, onStatus, onUsernameClick }: ModRoomPaneProps) => {
  const session = useModRoomStore((s) => s.sessions[channelId]);
  const draft = useModRoomStore((s) => s.drafts[channelId] ?? '');
  const storeSetDraft = useModRoomStore((s) => s.setDraft);

  const [connecting, setConnecting] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [showEmotes, setShowEmotes] = useState(false);
  const [replyingTo, setReplyingTo] = useState<ReplyRef | null>(null);
  const [editing, setEditing] = useState<{ id: string; attachment?: string; reply?: ReplyRef } | null>(null);
  const [showJump, setShowJump] = useState(false);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [, setTypingTick] = useState(0);

  const fontSize = useAppStore((s) => s.settings.chat_design?.font_size) ?? 14;
  const snRegVersion = useSyncExternalStore(
    subscribeStreamNookRegistryVersion,
    getStreamNookRegistryVersion,
    getStreamNookRegistryVersion,
  );

  const listRef = useRef<HTMLDivElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // Spell check for the mod-room composer. Everything runs locally against a
  // bundled dictionary — nothing about the draft leaves this process, which
  // matters here because the room itself is end-to-end encrypted.
  const spellcheckEnabled = useAppStore((s) => s.settings.chat_input?.spellcheck_enabled) ?? true;
  const spellUnderlayRef = useRef<HTMLDivElement>(null);
  const spellRanges = useSpellcheck(draft, { enabled: spellcheckEnabled, emoteKey: null });
  const lastTypingSent = useRef(0);
  const nearBottomRef = useRef(true);

  const smiley = useSwappingSmiley();

  const state = session?.state ?? 'connecting';
  const denial = session?.denial ?? null;
  const messages = session?.messages ?? EMPTY_MESSAGES;
  const members = session?.members ?? EMPTY_MEMBERS;
  const key = session?.key ?? null;
  const myUserId = session?.myUserId ?? '';
  const myLogin = session?.myLogin ?? '';
  const hasLoaded = session?.hasLoaded ?? false;
  const decrypted = session?.decrypted ?? {};
  const pending = session?.pending ?? {};
  const dividerTs = session?.dividerTs ?? 0;

  const setDraft = useCallback(
    (value: string) => storeSetDraft(channelId, value),
    [channelId, storeSetDraft],
  );

  // The room connection outlives this pane (the manager holds it for unread
  // tracking); mounting just adds a reference and flags the room as viewed.
  useEffect(() => {
    if (!channelId) return;
    ensureRoom(channelId);
    setViewing(channelId, true);
    nearBottomRef.current = true;
    setReplyingTo(null);
    setEditing(null);
    setShowJump(false);
    return () => {
      setViewing(channelId, false);
      releaseRoom(channelId);
    };
  }, [channelId]);

  // Expire stale typing entries (they only clear on a re-render tick).
  const typingUsers = session?.typingUsers ?? {};
  const typingActive = Object.keys(typingUsers).length > 0;
  useEffect(() => {
    if (!typingActive) return;
    const t = setInterval(() => setTypingTick((v) => v + 1), 1000);
    return () => clearInterval(t);
  }, [typingActive]);

  const handleListScroll = useCallback(() => {
    const el = listRef.current;
    if (!el) return;
    const near = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
    nearBottomRef.current = near;
    if (near) setShowJump(false);
  }, []);

  // Autoscroll only when already at the bottom (or the newest message is own);
  // otherwise offer a jump affordance instead of yanking the reader down.
  useEffect(() => {
    const el = listRef.current;
    if (!el || messages.length === 0) return;
    const last = messages[messages.length - 1];
    const own = !!myUserId && last.userId === myUserId;
    if (nearBottomRef.current || own) {
      el.scrollTop = el.scrollHeight;
    } else {
      setShowJump(true);
    }
  }, [messages, myUserId]);

  // Surface room state to the host so the chat header can show it (no sub-header).
  useEffect(() => {
    onStatus?.({ memberCount: members.length, encrypted: !!key, connected: state === 'connected', members });
  }, [members, key, state, onStatus]);

  // Once the StreamNook registry/theme catalog loads, re-resolve decorations for
  // everyone in view (their atmosphere/cologne/badge may have no-op'd before it
  // was ready). Keyed on the registry version so it only fires on a real load.
  useEffect(() => {
    messages.forEach((m) => ensureUser(m.userId, m.login));
    members.forEach((mm) => ensureUser(mm.userId, mm.login));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snRegVersion]);

  // Encrypt a message payload, dropping the embedded emote refs if the result
  // would exceed the server's body cap (viewers then fall back to their own
  // emote sets). Returns null only if even the ref-less payload is too big.
  const encryptPayload = useCallback(
    async (text: string, attachment?: string, reply?: ReplyRef, nonce?: string): Promise<string | null> => {
      if (!key) return null;
      const refs = collectEmoteRefs(text, emotes);
      const build = (withRefs: boolean) =>
        JSON.stringify({
          x: text,
          a: attachment,
          r: reply,
          e: withRefs && refs.length ? refs : undefined,
          n: nonce,
        });
      let token = await encryptText(key, build(true));
      if (token.length > MAX_TOKEN_CHARS) token = await encryptText(key, build(false));
      return token.length > MAX_TOKEN_CHARS ? null : token;
    },
    [key, emotes],
  );

  // Encrypt + send a new message optimistically: it renders immediately and
  // solidifies when the server echo (matched by the encrypted nonce) arrives.
  const sendNew = useCallback(
    async (text: string, attachment?: string, reply?: ReplyRef | null): Promise<boolean> => {
      if (!text && !attachment) return false;
      // Never emit plaintext from a mod room. The composer is disabled while
      // the key is missing, so this is just defense in depth.
      if (!key) return false;
      const clipped = text.slice(0, MAX_DRAFT); // insertEmote bypasses the textarea maxLength
      const nonce = crypto.randomUUID();
      const token = await encryptPayload(clipped, attachment, reply ?? undefined, nonce);
      if (!token) return false;
      const refs = collectEmoteRefs(clipped, emotes);
      const resolved: ResolvedPayload = {
        text: clipped,
        attachment,
        reply: reply ?? undefined,
        emotes: refs.length ? refs : undefined,
        n: nonce,
      };
      sendOptimistic(channelId, token, resolved);
      return true;
    },
    [key, emotes, channelId, encryptPayload],
  );

  const handleSend = useCallback(async () => {
    const text = draft.trim();
    if (editing) {
      // Edit: re-encrypt the new text, preserving the original attachment + reply,
      // and re-resolve the emotes used so they stay embedded. If there's text but
      // no key yet, keep the composer as-is rather than dropping the edit silently.
      if (text && !key) return;
      if (key && text) {
        const token = await encryptPayload(text.slice(0, MAX_DRAFT), editing.attachment, editing.reply);
        if (!token) return; // keep the editing state; the caps make this unreachable
        sendEdit(channelId, editing.id, token);
      }
      setEditing(null);
      setDraft('');
      return;
    }
    if (!text) return;
    if (!key) return; // wait until we can encrypt; the composer is disabled too
    const ok = await sendNew(text, undefined, replyingTo);
    if (!ok) return;
    setReplyingTo(null);
    setDraft('');
  }, [draft, editing, key, replyingTo, sendNew, encryptPayload, channelId, setDraft]);

  const handleDraftChange = (value: string) => {
    setDraft(value);
    setMentionIndex(0);
    const now = Date.now();
    if (now - lastTypingSent.current > TYPING_PING_MS) {
      lastTypingSent.current = now;
      sendTyping(channelId);
    }
  };

  const sendImageFile = async (file: File) => {
    // Don't upload/send attachments before encryption is ready: the bytes would
    // be stored in the clear. The attach button is disabled in this state too.
    if (!file || !key) return;
    setUploading(true);
    try {
      const body = await encryptBytes(key, new Uint8Array(await file.arrayBuffer()));
      const url = await uploadAttachment(channelId, body, 'application/x-sn-enc');
      const ok = await sendNew(draft.trim(), url, replyingTo);
      if (ok) {
        setReplyingTo(null);
        setDraft('');
      }
    } catch {
      // upload failed; leave the draft so the user can retry
    } finally {
      setUploading(false);
    }
  };

  const handleFilePick = async (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = '';
    if (file) await sendImageFile(file);
  };

  const handlePaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const file = Array.from(e.clipboardData?.files ?? []).find((f) => f.type.startsWith('image/'));
    if (file) {
      e.preventDefault();
      void sendImageFile(file);
    }
  };

  const handleConnectConsent = async () => {
    setConnecting(true);
    try {
      await connectModRoomConsent();
      retryRoom(channelId);
    } catch {
      // cancelled / failed; leave the CTA
    } finally {
      setConnecting(false);
    }
  };

  const startReply = (m: ModRoomChat, text: string) => {
    setEditing(null);
    setReplyingTo({ id: m.id, login: m.login, text: text.slice(0, 140) });
    textareaRef.current?.focus();
  };

  const startEdit = (m: ModRoomChat, text: string, attachment?: string, reply?: ReplyRef) => {
    setReplyingTo(null);
    setEditing({ id: m.id, attachment, reply });
    setDraft(text);
    textareaRef.current?.focus();
  };

  const cancelComposer = () => {
    setEditing(null);
    setReplyingTo(null);
    setDraft('');
  };

  const resolve = (
    m: ModRoomChat,
  ): { text: string; attachment?: string; reply?: ReplyRef; emotes?: EmoteRef[]; locked: LockState } => {
    if (!isEncrypted(m.body)) return { text: m.body, attachment: m.attachment, locked: false };
    const d = decrypted[m.id];
    if (key && d === null) return { text: '', locked: 'failed' };
    if (!key || d === undefined || d === null) return { text: '', locked: 'pending' };
    return { text: d.text, attachment: d.attachment, reply: d.reply, emotes: d.emotes, locked: false };
  };

  // ----- @mention autocomplete ------------------------------------------------

  const mentionMatch = /(^|\s)@(\w*)$/.exec(draft);
  const mentionCandidates = useMemo(() => {
    if (!mentionMatch) return [];
    const prefix = mentionMatch[2].toLowerCase();
    return members
      .filter((m) => m.login && m.login.toLowerCase().startsWith(prefix))
      .slice(0, 6);
  }, [mentionMatch, members]);
  const mentionOpen = mentionCandidates.length > 0;

  const insertMention = (login: string) => {
    if (!mentionMatch) return;
    const start = draft.slice(0, mentionMatch.index) + mentionMatch[1];
    setDraft(`${start}@${login} `);
    setMentionIndex(0);
    textareaRef.current?.focus();
  };

  const handleComposerKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (mentionOpen) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setMentionIndex((i) => (i + 1) % mentionCandidates.length);
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setMentionIndex((i) => (i - 1 + mentionCandidates.length) % mentionCandidates.length);
        return;
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        insertMention(mentionCandidates[Math.min(mentionIndex, mentionCandidates.length - 1)].login);
        return;
      }
      if (e.key === 'Escape') {
        // Break the @word so the popover closes without touching reply/edit state.
        e.preventDefault();
        setDraft(draft + ' ');
        return;
      }
    }
    if (e.key === 'Escape' && (editing || replyingTo)) {
      e.preventDefault();
      cancelComposer();
      return;
    }
    if (e.key === 'ArrowUp' && !draft && !editing) {
      // Edit the own most recent message, the muscle-memory chat-room gesture.
      const own = [...messages].reverse().find((m) => m.userId === myUserId && !pending[m.id]);
      if (own) {
        const r = resolve(own);
        if (!r.locked) {
          e.preventDefault();
          startEdit(own, r.text, r.attachment, r.reply);
        }
      }
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void handleSend();
    }
  };

  // ----- gated states -----------------------------------------------------------

  if (denial === 'needs_connect') {
    return (
      <CenterNote
        icon={<ShieldCheck size={28} className="text-accent" />}
        title="Connect mod rooms"
        body="Grant one-time access to the channels you moderate to join their private mod room."
        action={
          <MinimalButton onClick={handleConnectConsent} disabled={connecting}>
            {connecting ? 'Opening browser...' : 'Connect'}
          </MinimalButton>
        }
      />
    );
  }

  if (denial === 'not_entitled') {
    return (
      <CenterNote
        icon={<ShieldCheck size={28} className="text-[#f0c674]" />}
        title="Supporter feature"
        body="Mod rooms are available to StreamNook supporters and subscribers."
        action={
          <MinimalButton
            onClick={() =>
              openExternal(
                `https://streamnook.app/support?tier=supporter${channelLogin ? `&handle=${channelLogin}` : ''}`,
              )
            }
          >
            Become a supporter
          </MinimalButton>
        }
      />
    );
  }

  if (denial === 'not_moderator') {
    return (
      <CenterNote
        icon={<ShieldCheck size={28} className="text-textSecondary" />}
        title="No mod room here"
        body="You don't moderate this channel, so there's no room to join."
      />
    );
  }

  if (denial === 'error') {
    return (
      <CenterNote
        icon={<ShieldCheck size={28} className="text-textSecondary" />}
        title="Couldn't reach the mod room"
        body="Something went wrong connecting. Try again in a moment."
        action={<MinimalButton onClick={() => retryRoom(channelId)}>Retry</MinimalButton>}
      />
    );
  }

  const now = Date.now();
  const typingLogins = Object.values(typingUsers)
    .filter((t) => now - t.at < TYPING_TTL_MS)
    .map((t) => t.login)
    .filter(Boolean);

  let dividerShown = false;

  return (
    <div className="flex h-full flex-col">
      <div className="relative min-h-0 flex-1">
        <div ref={listRef} onScroll={handleListScroll} className="h-full overflow-y-auto py-1">
          {messages.length === 0 ? (
            <div className="flex h-full items-center justify-center px-6">
              {state === 'connected' && hasLoaded && (
                <p className="text-center text-sm text-textSecondary">No messages yet. Say hello to your mod team.</p>
              )}
            </div>
          ) : (
            messages.map((m, i) => {
              const prev = messages[i - 1];
              const dayChanged = prev
                ? new Date(prev.ts).toDateString() !== new Date(m.ts).toDateString()
                : new Date(m.ts).toDateString() !== new Date().toDateString();
              const showNew = !dividerShown && dividerTs > 0 && m.ts > dividerTs && m.userId !== myUserId;
              if (showNew) dividerShown = true;
              const { text, attachment, reply, emotes: msgEmotes, locked } = resolve(m);
              const pend = pending[m.id];
              const canEdit = !!myUserId && m.userId === myUserId && !locked && !pend;
              return (
                <Fragment key={m.id}>
                  {dayChanged && <HairlineDivider label={dayLabel(m.ts)} />}
                  {showNew && <HairlineDivider label="New" accent />}
                  <ModRoomMessageRow
                    m={m}
                    body={text}
                    attachment={attachment}
                    reply={reply}
                    emoteRefs={msgEmotes}
                    locked={locked}
                    canEdit={canEdit}
                    emotes={emotes}
                    fontSize={fontSize}
                    roomKey={key}
                    pending={!!pend}
                    sendFailed={pend?.failed}
                    mentioned={!locked && m.userId !== myUserId && mentionsMe(text, myLogin)}
                    onUsernameClick={onUsernameClick}
                    onReply={() => startReply(m, text)}
                    onEdit={() => startEdit(m, text, attachment, reply)}
                    onRetry={() => retryPending(channelId, m.id)}
                    onDiscard={() => {
                      discardPending(channelId, m.id);
                      if (!draft) setDraft(text);
                    }}
                  />
                </Fragment>
              );
            })
          )}
        </div>
        {showJump && (
          <button
            onClick={() => {
              const el = listRef.current;
              if (el) el.scrollTop = el.scrollHeight;
              nearBottomRef.current = true;
              setShowJump(false);
            }}
            className="absolute bottom-2 right-3 z-20 rounded-full px-2.5 py-1 text-[11px] font-medium text-textPrimary transition-colors hover:text-accent"
            style={{ background: 'rgba(24,24,26,0.92)', boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.08)' }}
          >
            Jump to latest
          </button>
        )}
      </div>

      <div className="h-4 px-3 text-[11px] text-textSecondary">
        {typingLogins.length === 1 && `${typingLogins[0]} is typing...`}
        {typingLogins.length === 2 && `${typingLogins[0]} and ${typingLogins[1]} are typing...`}
        {typingLogins.length > 2 && 'Several people are typing...'}
      </div>

      <div className="relative flex-shrink-0 border-t border-borderSubtle p-2" style={{ backgroundColor: 'rgba(12, 12, 13, 0.9)' }}>
        <EmotePickerPanel
          open={showEmotes}
          onClose={() => setShowEmotes(false)}
          emotes={emotes ?? null}
          isTwitch
          isKick={false}
          channelId={channelId}
          channelLogin={channelLogin}
          onInsert={(name) => {
            setDraft(draft + (draft && !draft.endsWith(' ') ? ' ' : '') + name + ' ');
            textareaRef.current?.focus();
          }}
        />
        {mentionOpen && (
          <div
            className="absolute bottom-full left-2 z-30 mb-1 min-w-[160px] overflow-hidden rounded-lg py-1"
            style={{ background: 'rgba(18,18,20,0.98)', boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.08)' }}
          >
            {mentionCandidates.map((c, i) => (
              <button
                key={c.userId}
                onClick={() => insertMention(c.login)}
                className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs transition-colors hover:bg-surface-hover ${i === mentionIndex ? 'bg-white/[0.06] text-textPrimary' : 'text-textSecondary'}`}
              >
                <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${c.active !== false ? 'bg-accent' : 'bg-white/25'}`} />
                <span className="truncate">{c.login}</span>
              </button>
            ))}
          </div>
        )}
        {(replyingTo || editing) && (
          <div
            className="mb-2 flex items-center gap-2 rounded-md px-2 py-1 text-[11px]"
            style={
              editing
                ? { background: 'rgba(245,158,11,0.14)', boxShadow: 'inset 0 0 0 1px rgba(245,158,11,0.32)' }
                : { background: 'rgba(255,255,255,0.05)' }
            }
          >
            {editing ? (
              <span className="flex shrink-0 items-center gap-1 font-semibold text-amber-300">
                <Pencil size={11} /> Editing message
              </span>
            ) : (
              <span className="flex shrink-0 items-center gap-1 text-textSecondary">
                <CornerUpLeft size={11} /> Replying to <span className="text-accent">{replyingTo!.login}</span>
              </span>
            )}
            {replyingTo && <span className="truncate text-textSecondary opacity-70">{replyingTo.text}</span>}
            <button
              onClick={cancelComposer}
              aria-label="Cancel"
              className="ml-auto shrink-0 text-textSecondary transition-colors hover:text-textPrimary"
            >
              <X size={13} />
            </button>
          </div>
        )}
        <div className="flex items-end gap-1.5">
          <input ref={fileInputRef} type="file" accept="image/*" className="hidden" onChange={handleFilePick} />
          <button
            onClick={() => fileInputRef.current?.click()}
            disabled={state !== 'connected' || !key || uploading}
            aria-label="Attach image"
            className="grid h-[34px] w-[34px] place-items-center rounded text-textSecondary transition-colors hover:bg-surface-hover hover:text-accent disabled:opacity-40"
          >
            <Paperclip size={16} />
          </button>
          <button
            onClick={() => setShowEmotes((v) => !v)}
            onMouseLeave={smiley.cycleEmoteSmiley}
            disabled={state !== 'connected' || !key}
            aria-label="Emotes"
            className="group grid h-[34px] w-[34px] place-items-center rounded transition-colors hover:bg-surface-hover disabled:opacity-40"
          >
            {showEmotes ? (
              <svg className="h-4 w-4 text-accent" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2.5} d="M6 18L18 6M6 6l12 12" />
              </svg>
            ) : (
              <img
                src={getAppleEmojiUrl(smiley.currentSmiley)}
                alt={smiley.currentSmiley}
                draggable={false}
                className={`h-4 w-4 object-contain transition-all ease-in-out ${smiley.isSmileyTransitioning ? 'scale-50 opacity-0 duration-100' : 'scale-100 opacity-100 duration-150'}`}
              />
            )}
          </button>
          {/* Wrapper so the spell-check underlay can lay over the textarea. */}
          <div className="relative flex min-w-0 flex-1">
          <textarea
            ref={textareaRef}
            value={draft}
            maxLength={MAX_DRAFT}
            onChange={(e) => handleDraftChange(e.target.value)}
            onKeyDown={handleComposerKeyDown}
            onPaste={handlePaste}
            onFocus={warmSpellcheck}
            spellCheck={false}
            data-spellcheck={spellcheckEnabled ? 'true' : undefined}
            onScroll={(e) => {
              if (spellUnderlayRef.current) {
                spellUnderlayRef.current.scrollTop = e.currentTarget.scrollTop;
              }
            }}
            rows={1}
            placeholder={state !== 'connected' ? 'Connecting...' : key ? 'Encrypted message' : 'Securing room...'}
            disabled={state !== 'connected' || !key}
            className="glass-input max-h-28 min-h-[34px] w-full resize-none px-3 py-2 text-sm placeholder-textSecondary"
          />
          {spellRanges.length > 0 && (
            <SpellcheckUnderlay
              innerRef={spellUnderlayRef}
              text={draft}
              ranges={spellRanges}
              className="px-3 py-2 text-sm"
            />
          )}
          </div>
          <button
            onClick={() => void handleSend()}
            disabled={state !== 'connected' || !key || !draft.trim()}
            aria-label="Send"
            className="glass-button flex h-9 w-9 flex-shrink-0 items-center justify-center self-center rounded text-white transition-all duration-300 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" />
            </svg>
          </button>
        </div>
      </div>
    </div>
  );
};

function sniffImageType(b: Uint8Array): string {
  if (b[0] === 0x89 && b[1] === 0x50) return 'image/png';
  if (b[0] === 0xff && b[1] === 0xd8) return 'image/jpeg';
  if (b[0] === 0x47 && b[1] === 0x49) return 'image/gif';
  if (b[0] === 0x52 && b[1] === 0x49 && b[8] === 0x57) return 'image/webp';
  return 'image/png';
}

// Fetches an attachment and, if it was stored encrypted (application/x-sn-enc),
// decrypts it with the room key into a blob URL. Legacy plaintext images render
// as-is. The bytes never sit decrypted anywhere but this client. Click to expand.
const AttachmentImage = ({ url, roomKey }: { url: string; roomKey: CryptoKey | null }) => {
  const [src, setSrc] = useState<string | null>(null);
  // Keyed by url so a re-run for a new url resets failure without a sync setState.
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  const failed = failedUrl === url;
  useEffect(() => {
    let active = true;
    let blobUrl: string | null = null;
    void (async () => {
      try {
        const res = await fetch(url);
        if (!res.ok) {
          if (active) setFailedUrl(url);
          return;
        }
        const ct = res.headers.get('Content-Type') || '';
        let bytes = new Uint8Array(await res.arrayBuffer());
        if (ct.includes('x-sn-enc')) {
          if (!roomKey) return; // key not here yet; the effect re-runs when it lands
          const dec = await decryptBytes(roomKey, bytes);
          if (!dec) {
            if (active) setFailedUrl(url);
            return;
          }
          bytes = dec;
        }
        blobUrl = URL.createObjectURL(new Blob([bytes], { type: sniffImageType(bytes) }));
        if (active) setSrc(blobUrl);
        else URL.revokeObjectURL(blobUrl);
      } catch {
        if (active) setFailedUrl(url);
      }
    })();
    return () => {
      active = false;
      if (blobUrl) URL.revokeObjectURL(blobUrl);
    };
  }, [url, roomKey]);
  if (failed) return <p className="text-[11px] italic text-textSecondary">Attachment unavailable</p>;
  if (!src) return <div className="h-24 w-40 animate-pulse rounded-md bg-white/5" />;
  return (
    <>
      <img
        src={src}
        alt="attachment"
        onClick={() => setExpanded(true)}
        className="max-h-48 max-w-[85%] cursor-zoom-in rounded-md object-contain"
      />
      {expanded && (
        <div
          className="fixed inset-0 z-50 flex cursor-zoom-out items-center justify-center bg-black/80"
          onClick={() => setExpanded(false)}
        >
          <img src={src} alt="attachment" className="max-h-[90vh] max-w-[90vw] rounded-md object-contain" />
        </div>
      )}
    </>
  );
};

const CenterNote = ({
  icon,
  title,
  body,
  action,
}: {
  icon: ReactNode;
  title: string;
  body: string;
  action?: ReactNode;
}) => (
  <div className="flex h-full flex-col items-center justify-center gap-3 px-8 text-center">
    {icon}
    <div>
      <p className="text-sm font-semibold text-textPrimary">{title}</p>
      <p className="mt-1 text-xs text-textSecondary">{body}</p>
    </div>
    {action}
  </div>
);

const MinimalButton = ({
  children,
  onClick,
  disabled,
}: {
  children: ReactNode;
  onClick: () => void;
  disabled?: boolean;
}) => (
  <button
    onClick={onClick}
    disabled={disabled}
    className="px-3 py-1.5 text-sm font-medium text-textSecondary transition-colors hover:text-accent disabled:opacity-50"
  >
    {children}
  </button>
);

export default ModRoomPane;
