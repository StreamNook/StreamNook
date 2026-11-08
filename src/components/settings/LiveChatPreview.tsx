// Settings > Chat's pinned chat preview (unlabelled: it is simply chat). Rust
// builds each scene's rows from real Twitch chat lines through the path a live
// line takes (services/chat_preview.rs: parse, your rules, the row view), and
// the REAL chat row (ChatMessage) draws them inside the same column chat uses.
// So a cheer, a raid, an emote or a reply looks here exactly as chat draws it,
// and every setting shows the moment it is saved. The rows follow the section
// being read (names and badges, events, mentions, links, emotes, hidden
// messages, highlights). The made-up chatters' 7TV cosmetics (not part of a
// chat line) come from the overlay builder's samples through
// PreviewChattersContext, never the chat-user store real chat completes from.
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { ScrollbarMarks } from '../chat/ScrollbarMarks';
import { collectMarks, FALLBACK_MARK_KINDS } from '../chat/scrollbarMarkLayout';
import { IS_MAC } from '../../utils/platform';
import { invoke } from '@tauri-apps/api/core';
import ChatMessage from '../ChatMessage';
import { useAppStore } from '../../stores/AppStore';
import { useChatUserStore } from '../../stores/chatUserStore';
import { SAMPLE_MESSAGES } from '../overlay/sampleMessages';
import type { BackendChatMessage } from '../../services/twitchChat';
import { useFreshIds } from '../../hooks/useFreshIds';
import { PreviewChattersContext, type PreviewChatter } from '../chat/previewChatters';
import { listAtmospheres } from '../../services/atmospheres';
import { useSectionInView } from './sectionInView';
import { lineButtons, type LineButton } from '../chat/modLine';
import { bindLineStrips, useLineArt, useLineButtons } from '../chat/lineArt';
import { useLineConfig, useModSliderSteps, useSliderConfig, type ModStep } from '../../services/moderation';

const NO_STEPS: ModStep[] = [];
import { initializeBadgeCache } from '../../services/twitchBadges';
import { ensureChannelEmotes } from '../../stores/chatConnectionStore';
import type { BetView, PollView, PredictionView } from '../../types';
import type { RoomModeView } from '../../stores/chatConnectionStore';
import { PreviewComposer } from './PreviewComposer';
import { PinnedBar, type PinnedMessage } from '../chat/PinnedBar';
import PollOverlay from '../PollOverlay';
import PredictionOverlay from '../PredictionOverlay';
import { ChatOverlayStack } from '../chat/ChatOverlayStack';
import { useChatTopStack } from '../chat/chatTopStack';

type Moderation = { type: 'timeout' | 'ban' | 'deleted'; duration?: number };

/** Rust's answer (services/chat_preview.rs PreviewChat). */
interface PreviewChat {
  rows: BackendChatMessage[];
  poll: PollView | null;
  prediction: PredictionView | null;
  prediction_bet: BetView | null;
  pins: PinnedMessage[];
  room_modes: RoomModeView | null;
}

/** Which part of the message box a section shows off. */
const COMPOSER_OF: Record<string, string> = {
  'Chat Input': 'input',
  'Tab Completion': 'tab',
  Drops: 'drops',
};

type Scene = 'conversation' | 'events' | 'names' | 'mentions' | 'links' | 'emotes' | 'hidden' | 'highlights' | 'polls' | 'pinned';

/** Which rows each section needs on screen for its settings to show. */
const SCENE_OF: Record<string, Scene> = {
  'Removed Messages': 'hidden',
  'Polls & Predictions': 'polls',
  'Pinned Messages': 'pinned',
  'Chat Events': 'events',
  'Event Highlights': 'events',
  'Channel Points': 'events',
  'Names & Badges': 'names',
  Nicknames: 'names',
  'Mentions & Replies': 'mentions',
  Links: 'links',
  Emotes: 'emotes',
  'Emote Effects': 'emotes',
  'Chat Behavior': 'hidden',
  'Hidden Messages': 'hidden',
  'Highlight Appearance': 'highlights',
  'Highlight Phrases': 'highlights',
  'Username Highlights': 'highlights',
  'Badge Highlights': 'highlights',
};

/** What moderation did to a sample row (chat learns it from CLEARMSG and
 *  CLEARCHAT, which the list applies to the row as this context). */
const MODERATED: Record<string, Moderation> = {
  'lc-deleted': { type: 'deleted' },
  'lc-timeout': { type: 'timeout', duration: 600 },
};

/** The made-up chatters' 7TV cosmetics and membership, from the overlay
 *  builder's samples, keyed by the user id their chat lines carry. */
function sampleChatters(): Map<string, PreviewChatter> {
  const aurora = listAtmospheres().find((a) => /aurora/i.test(a.name ?? a.id)) ?? listAtmospheres()[0];
  const out = new Map<string, PreviewChatter>();
  for (const m of SAMPLE_MESSAGES) {
    if (!m.paint && !m.seventvBadgeUrl && !m.extraBadges?.length && !m.atmosphere) continue;
    const badgeId = m.seventvBadgeUrl?.match(/\/badge\/([^/]+)\//)?.[1];
    // Only a StreamNook member can wear an atmosphere, and a member's row wears
    // the member badge; the samples stay to what chat could really show.
    const member = m.streamNookUserNumber != null;
    out.set(`uid-${m.username.replace(/^@/, '')}`, {
      streamNookMember: member,
      paint: m.paint,
      seventvBadge: badgeId ? { id: badgeId, name: m.seventvBadgeTitle ?? '7TV', tooltip: m.seventvBadgeTitle ?? '7TV', images: [] } : undefined,
      thirdPartyBadges: (m.extraBadges ?? []).map((b, i) => ({
        id: `${m.id}-${i}`,
        title: b.title ?? '',
        imageUrl: b.url,
        image1x: b.url,
        image2x: b.url,
        image4x: b.url,
        provider: b.source ?? '',
        category: b.category,
      })),
      atmosphereId: member && m.atmosphere && aurora ? aurora.id : null,
    });
  }
  return out;
}

/** Swallows every press, click and key inside the preview before it reaches a
 *  control, so a moderator preview's buttons can never moderate anyone (the
 *  sample names may be real Twitch users). Hover still works. */
const SWALLOW = {
  onPointerDownCapture: (e: React.SyntheticEvent) => {
    e.preventDefault();
    e.stopPropagation();
  },
  onMouseDownCapture: (e: React.SyntheticEvent) => {
    e.preventDefault();
    e.stopPropagation();
  },
  onClickCapture: (e: React.SyntheticEvent) => {
    e.preventDefault();
    e.stopPropagation();
  },
  onKeyDownCapture: (e: React.SyntheticEvent) => {
    e.preventDefault();
    e.stopPropagation();
  },
};

/**
 * `moderation`: Settings > Moderation's preview. The rows are drawn as a
 * moderator of your chat sees them, with the swipe edge, line buttons and
 * hover buttons your settings choose, built by chat's own hooks; nothing in
 * it can be pressed.
 */
export function LiveChatPreview({ moderation = false }: { moderation?: boolean } = {}) {
  const settings = useAppStore((s) => s.settings);
  const design = settings.chat_design;
  const user = useAppStore((s) => s.currentUser);
  const section = useSectionInView();
  const scene: Scene = (section && SCENE_OF[section]) || 'conversation';
  // Your colour as chat knows it (the chat-user store holds what Twitch sent).
  const knownColor = useChatUserStore((s) => (user?.user_id ? s.users.get(user.user_id)?.color : undefined));
  const youName = user?.display_name || user?.login || '';
  const youLogin = (user?.login || user?.username || '').toLowerCase();
  // The rows are always parsed for your own chat (its emotes and badges),
  // whatever is playing, so the preview never changes with the stream.
  const channel = youLogin || 'streamnook';
  const roomId = user?.user_id || '';

  // Each saved change of the chat look: lights the column, and replays arrival
  // effects (the mention line comes back with a fresh send time).
  const sig = JSON.stringify(design ?? {});
  const changed = useFreshIds([sig]).has(sig);
  const chatters = useMemo(() => sampleChatters(), []);

  // Your line draws your cosmetics from the chat-user store, the way chat does
  // once you have spoken; register yourself the same way chat does on join.
  useEffect(() => {
    if (!user?.user_id) return;
    const store = useChatUserStore.getState();
    if (!store.users.get(user.user_id)) {
      store.addUser({ userId: user.user_id, username: youLogin, displayName: youName, color: knownColor || '#9147FF' });
    }
  }, [user?.user_id, youLogin, youName, knownColor]);

  // Your highlights to show off: the first plain phrase, user and badge.
  const hl = settings.chat_highlights;
  const phrase = hl?.phrases?.find((p) => p.enabled && !p.is_regex && p.pattern.trim())?.pattern ?? null;
  const highlightUser = hl?.users?.find((u) => u.enabled && u.username.trim())?.username ?? null;
  const highlightBadge = hl?.badges?.find((b) => b.enabled && b.badge_key.includes('/'))?.badge_key ?? null;

  // Twitch badge art comes from the badge cache chat fills when it opens (the
  // overlay preview loads it the same way); rows are fetched again once it has.
  // The chat's emotes too, through the cache chat shares between panes; with
  // them loaded Rust turns 7TV/BTTV/FFZ words into emotes and Tab finds them.
  const [badgesReady, setBadgesReady] = useState(0);
  useEffect(() => {
    let live = true;
    void initializeBadgeCache(roomId || undefined).then(() => {
      if (live) setBadgesReady((n) => n + 1);
    });
    void ensureChannelEmotes(channel, roomId, 'twitch')
      .then(() => {
        if (live) setBadgesReady((n) => n + 1);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [roomId, channel]);

  const [preview, setPreview] = useState<PreviewChat>({
    rows: [],
    poll: null,
    prediction: null,
    prediction_bet: null,
    pins: [],
    room_modes: null,
  });
  const rows = preview.rows;
  useEffect(() => {
    let live = true;
    // A beat after a change, so Rust has rebuilt its rules from the save.
    const timer = setTimeout(() => {
      invoke<PreviewChat>('preview_chat_rows', {
        ask: {
          scene,
          channel,
          room_id: roomId,
          you_login: youLogin,
          you_id: user?.user_id ?? '',
          you_name: youName,
          you_color: knownColor ?? '',
          fresh: changed,
          phrase,
          highlight_user: highlightUser,
          highlight_badge: highlightBadge,
        },
      })
        .then((next) => {
          if (live) setPreview(next);
        })
        .catch(() => {});
    }, 150);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [scene, channel, roomId, youLogin, user?.user_id, youName, knownColor, changed, phrase, highlightUser, highlightBadge, settings, badgesReady]);

  // The same column variables the chat list sets (ChatMessageList's scroll
  // root), from the same settings.
  const vars = {
    '--sn-emote-scale': design?.emote_scale ?? 1,
    '--sn-gif-scale': design?.gif_scale ?? 1,
    '--sn-emote-word-scale': design?.emote_words_scale ?? 1,
    '--sn-emote-margin': `${design?.emote_margin ?? 0.125}rem`,
    '--sn-backfill-opacity': Math.max(0.3, Math.min(1, (design?.backfill_opacity ?? 100) / 100)),
    '--sn-badge-scale': Math.max(0.5, Math.min(2.5, design?.badge_scale ?? 1)),
  } as CSSProperties;
  const entrance = design?.message_entrance && design.message_entrance !== 'none' ? design.message_entrance : undefined;

  // Moderator line buttons, resolved by the hooks chat's list uses
  // (ChatMessageList): Rust's steps for your line list, drawn once for the
  // list. Pressing one does nothing here.
  const lineOn = moderation && design?.mod_line_buttons === true;
  const lineConfig = useLineConfig();
  const lineSteps = useModSliderSteps('twitch', lineConfig, lineOn);
  const modLine = useMemo<LineButton[] | null>(
    () => (lineOn && lineSteps && lineSteps.length > 0 ? lineButtons(lineSteps) : null),
    [lineOn, lineSteps],
  );
  const lineShow = design?.mod_line_show === 'always' ? 'always' : 'hover';
  const sliderConfig = useSliderConfig();
  const ladderSteps = useModSliderSteps('twitch', sliderConfig, lineOn);
  const lineLadder = useMemo(() => ladderSteps?.filter((s) => s.action === 'timeout') ?? NO_STEPS, [ladderSteps]);
  const { buttons: lineNow } = useLineButtons(lineOn ? lineSteps : null, lineLadder);
  const lineFit = useMemo(
    () => [...lineLadder, ...(lineSteps ?? []).filter((s) => s.action === 'timeout')].map((s) => s.label),
    [lineLadder, lineSteps],
  );
  const lineArt = useLineArt(lineNow, design?.font_size ?? 14, lineFit, modLine);
  const lineArtRef = useRef<{ art: NonNullable<typeof lineArt>; buttons: LineButton[] } | null>(null);
  useLayoutEffect(() => {
    lineArtRef.current = lineArt && lineNow ? { art: lineArt, buttons: lineNow } : null;
  });

  // Scrollbar marks, collected the way the chat list collects them (Rust
  // stamps each row's mark; your kinds choose which show).
  const marksOn = design?.scrollbar_marks ?? true;
  const markKinds = design?.scrollbar_mark_kinds;
  const marks = useMemo(
    () =>
      collectMarks(
        rows,
        (m) => (typeof m === 'string' ? null : m.id),
        new Set<string>(marksOn ? (markKinds ?? FALLBACK_MARK_KINDS) : []),
      ),
    [rows, marksOn, markKinds],
  );
  // Like chat, the list sits on its newest line.
  const scrollerRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = scrollerRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [rows, scene]);
  // The line buttons light up under the pointer as in chat; a press does nothing.
  useEffect(() => {
    const el = scrollerRef.current;
    if (!el || !modLine) return;
    return bindLineStrips(el, () => lineArtRef.current, () => {});
  }, [modLine]);

  // Poll and prediction cards float over the top of the list in chat's own
  // stack, laid out by Rust from your card order and fold settings.
  const [column, setColumn] = useState<HTMLDivElement | null>(null);
  const topStack = useChatTopStack(column);
  const showPredictions = settings.show_predictions !== false;
  const showPolls = settings.show_polls !== false;
  // The pinned bar opens or starts folded as chat's does, and again whenever
  // that setting changes; a click toggles it like chat's.
  const pinStartsCollapsed = design?.pinned_start_collapsed ?? true;
  const [pinOpen, setPinOpen] = useState(!pinStartsCollapsed);
  const [pinSetting, setPinSetting] = useState(pinStartsCollapsed);
  if (pinSetting !== pinStartsCollapsed) {
    setPinSetting(pinStartsCollapsed);
    setPinOpen(!pinStartsCollapsed);
  }

  return (
    // The chat panel's own surface (the app's chat column is plain
    // bg-background, square, unframed) at the default chat width, so lines wrap
    // where chat's do.
    <div ref={setColumn} className="relative w-full max-w-[402px] overflow-hidden bg-background" {...(moderation ? SWALLOW : {})}>
      <PinnedBar
        pins={preview.pins}
        expanded={pinOpen}
        onExpand={() => setPinOpen(true)}
        onCollapse={() => setPinOpen(false)}
        provider="twitch"
        channelId={roomId || undefined}
        top={0}
        renderText={(text) => text}
      />
      {(preview.poll || preview.prediction) && (
        <ChatOverlayStack top={8} maxHeight={topStack.maxHeight}>
          {showPredictions && preview.prediction && (
            <PredictionOverlay
              key="prediction"
              previewView={preview.prediction}
              previewBet={preview.prediction_bet ?? undefined}
              stack={topStack.control.prediction}
            />
          )}
          {showPolls && preview.poll && <PollOverlay key="poll" previewView={preview.poll} stack={topStack.control.poll} />}
        </ChatOverlayStack>
      )}
      <PreviewChattersContext.Provider value={chatters}>
        {/* Chat's own scrolling list (ChatMessageList's scroll root classes),
            as tall as the window allows, on its newest line. */}
        <div
          ref={scrollerRef}
          data-scroll-root=""
          className="relative max-h-[calc(100vh-8.5rem)] overflow-y-auto overflow-x-hidden scrollbar-thin"
          style={{ ...vars, ...lineArt?.vars }}
          data-line-show={modLine ? lineShow : undefined}
        >
          <div
            key={scene}
            ref={contentRef}
            className={`sn-preview-scene flex min-h-full flex-col justify-end${design?.alternating_backgrounds ? ' chat-striped' : ''}`}
            data-entrance={entrance}
          >
            {rows.map((message, i) => (
              <div
                // Arrival effects replay on a look change: the newest line (the
                // message animation) and the mention (its flash).
                key={message.id === 'lc-mention' || i === rows.length - 1 ? `${message.id}-${sig}` : message.id}
                dir="ltr"
                data-message-id={message.id}
                className="chat-message-row"
              >
                <ChatMessage
                  message={message}
                  moderationContext={MODERATED[message.id] ?? null}
                  emotes={null}
                  homeProvider="twitch"
                  isModerator={moderation}
                  broadcasterId={moderation ? roomId : undefined}
                  modLine={modLine}
                />
              </div>
            ))}
          </div>
        </div>
        {marksOn && <ScrollbarMarks scrollerRef={scrollerRef} contentRef={contentRef} marks={marks} overlay={IS_MAC} />}
        <PreviewComposer
          section={(section && COMPOSER_OF[section]) || ''}
          roomModes={preview.room_modes}
          channel={channel}
          roomId={roomId}
          emotesReady={badgesReady}
        />
      </PreviewChattersContext.Provider>
      {changed && <span key={sig} aria-hidden className="sn-preview-changed pointer-events-none absolute inset-0" />}
    </div>
  );
}
