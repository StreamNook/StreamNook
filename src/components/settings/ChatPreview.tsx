import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from 'react';
import { Gift, Inbox, RotateCcw } from 'lucide-react';
import { DotsThreeVertical } from 'phosphor-react';
import { useNameColorAdjust } from '../../hooks/useNameColor';
import { StyledChatName, type NameSeparator, type NameStyle } from '../chat/StyledChatName';
import { deletedBodyStyle, deletedRowDimmed, type DeletedMessageStyle } from '../chat/deletedMessage';
import { ModerationTag } from '../chat/ModerationTag';
import { eventCardClass, eventCardStyle } from '../chat/eventCard';
import type { ModerationContext, RoomModeView } from '../../stores/chatConnectionStore';
import { RoomModeIndicator } from '../chat/RoomModeIndicator';
import { mentionHoldBackground } from '../chat/mentionHold';
import type { ChatEventSettings } from '../../types';
import { Tooltip } from '../ui/Tooltip';
import { calculateHalfPadding } from '../../utils/chatLayoutUtils';
import { chatBodyWeight } from '../../utils/chatBodyWeight';
import { mentionBoxClass, mentionBoxStyle, mentionTextStyle, type MentionLook } from '../chat/mentionStyle';
import { ScrollbarMarks } from '../chat/ScrollbarMarks';
import { collectMarks } from '../chat/scrollbarMarkLayout';
import type { BackendChatMessage, ScrollbarMarkKind } from '../../services/twitchChat';
import type { ChatDesignSettings } from '../../types';
import { ModSlider } from '../chat/ModSlider';
import { ModLineButtons } from '../chat/ModLineButtons';
import { HoverModButtons } from '../chat/HoverModButtons';
import { Copy, PushPin } from 'phosphor-react';
import { formatDuration } from '../../utils/timeoutRamp';
import { lineButtons, type LineButton } from '../chat/modLine';
import { bindLineStrips, bindLineWheel, useLineArt, useLineButtons, type LineArt } from '../chat/lineArt';
import { useLineConfig, useModSliderSteps, useSliderConfig } from '../../services/moderation';

// A few lines of sample chat for the settings page, drawn with the viewer's own
// chat design (text size, spacing, dividers, stripes, timestamps, name style)
// and the same helpers the real chat row uses, so a setting whose name cannot
// say what it looks like can show it instead.

/** The resolved chat design the preview draws with (ChatSettings' `cd`). */
export interface PreviewDesign {
  show_dividers: boolean;
  alternating_backgrounds: boolean;
  message_spacing: number;
  font_size: number;
  font_weight: number;
  show_timestamps: boolean;
  show_timestamp_seconds: boolean;
  timestamp_format: '12h' | '24h';
  username_separator: NameSeparator;
  username_style: NameStyle;
  username_accent_source: 'user' | 'theme';
}

interface Chatter {
  name: string;
  color: string;
}

const PIXELFOX: Chatter = { name: 'pixelfox', color: '#1E90FF' };
const MOSSY: Chatter = { name: 'mossy', color: '#2E8B57' };
const LOOTGOBLIN: Chatter = { name: 'lootgoblin', color: '#DAA520' };
const QUIETSTORM: Chatter = { name: 'quietstorm', color: '#DA70D6' };

// One fixed evening, so the sample clock never jumps while settings change.
const SAMPLE_TIME = new Date(2026, 0, 1, 19, 42, 30);

const sampleTime = (design: PreviewDesign, minutesAgo: number) =>
  new Date(SAMPLE_TIME.getTime() - minutesAgo * 60_000).toLocaleTimeString('en-US', {
    hour: 'numeric',
    minute: '2-digit',
    second: design.show_timestamp_seconds ? '2-digit' : undefined,
    hour12: design.timestamp_format !== '24h',
  });

/** The "Preview" heading over a sample, with an optional replay for settings
 *  that animate once. */
export const SettingPreview = ({
  caption,
  onReplay,
  children,
}: {
  caption?: ReactNode;
  onReplay?: () => void;
  children: ReactNode;
}) => (
  <div className="space-y-2">
    <div className="flex items-baseline gap-2">
      <span className="text-xs font-medium text-textSecondary uppercase tracking-wider">Preview</span>
      {caption && <span className="min-w-0 text-[11px] text-textMuted">{caption}</span>}
      {onReplay && (
        <Tooltip content="Play again">
          <button
            type="button"
            onClick={onReplay}
            aria-label="Play the preview again"
            className="ml-auto inline-flex self-center text-textMuted transition-colors hover:text-textPrimary"
          >
            <RotateCcw size={12} />
          </button>
        </Tooltip>
      )}
    </div>
    {children}
  </div>
);

/** The chat column the sample lines sit in. */
const PreviewChat = ({
  design,
  entrance,
  children,
}: {
  design: PreviewDesign;
  entrance?: string;
  children: ReactNode;
}) => (
  <div className="glass-panel overflow-hidden rounded-lg py-1">
    <div
      className={`flex flex-col${design.alternating_backgrounds ? ' chat-striped' : ''}`}
      data-entrance={entrance && entrance !== 'none' ? entrance : undefined}
    >
      {children}
    </div>
  </div>
);

/** The padding and divider a chat row carries (ChatMessage's row box). */
const rowBox = (design: PreviewDesign, first: boolean) => ({
  className: `px-3${design.show_dividers && !first ? ' border-t border-borderSubtle' : ''}`,
  style: {
    paddingTop: `${calculateHalfPadding(design.message_spacing)}px`,
    paddingBottom: `${calculateHalfPadding(design.message_spacing)}px`,
  } as CSSProperties,
});

const PreviewLine = ({
  design,
  who,
  text,
  minutesAgo,
  first = false,
  moderation,
  deletedStyle,
  action = false,
  lead,
}: {
  design: PreviewDesign;
  who: Chatter;
  text: ReactNode;
  minutesAgo: number;
  first?: boolean;
  moderation?: ModerationContext;
  deletedStyle?: DeletedMessageStyle;
  /** A /me line: the whole body in the sender's colour and italic, as chat draws it. */
  action?: boolean;
  /** Drawn at the start of the name line, as chat draws the line buttons. */
  lead?: ReactNode;
}) => {
  const adjust = useNameColorAdjust();
  const color = adjust(who.color) ?? who.color;
  const box = rowBox(design, first);
  const dim = moderation && deletedStyle && deletedRowDimmed(deletedStyle);
  return (
    <div
      className={`${box.className} transition-opacity duration-200`}
      // The row carries the chat font size, as the real row does, so the
      // name and text share one baseline.
      style={{ ...box.style, opacity: dim ? 0.5 : undefined, fontSize: `${design.font_size}px`, lineHeight: 1.625 }}
    >
      {design.show_timestamps && (
        <div className="mb-0.5 text-[10px] leading-tight text-textSecondary opacity-50">
          {sampleTime(design, minutesAgo)}
        </div>
      )}
      <span
        className="leading-relaxed"
        style={{ fontSize: `${design.font_size}px`, fontWeight: design.font_weight }}
      >
        {lead}
        <StyledChatName
          name={who.name}
          nameTextStyle={{ color }}
          nameStyle={design.username_style}
          separator={design.username_separator}
          accentColor={design.username_accent_source === 'theme' ? 'var(--color-accent)' : color}
        />
        <span
          style={action ? { color, fontWeight: chatBodyWeight(design.font_weight) } : { fontWeight: chatBodyWeight(design.font_weight) }}
          className={action ? 'italic break-words' : 'text-textPrimary break-words'}
        >
          <span data-mod-body style={moderation && deletedStyle ? deletedBodyStyle(deletedStyle) : undefined}>
            {' '}
            {text}
          </span>
          {moderation && deletedStyle && <ModerationTag context={moderation} style={deletedStyle} />}
        </span>
      </span>
    </div>
  );
};

/** A plain chat line as a list child (what striping and arrival key off).
 *  `open` lets a line draw past its box (the mod slider's chip); such a line
 *  never collapses. */
const Row = ({ children, collapsed = false, open = false }: { children: ReactNode; collapsed?: boolean; open?: boolean }) => (
  <div
    className="chat-message-row grid transition-[grid-template-rows,opacity] duration-300 ease-out"
    style={{ gridTemplateRows: collapsed ? '0fr' : '1fr', opacity: collapsed ? 0 : 1 }}
    aria-hidden={collapsed || undefined}
  >
    <div className={open ? 'min-h-0' : 'min-h-0 overflow-hidden'}>{children}</div>
  </div>
);

const TIMEOUT: ModerationContext = { type: 'timeout', duration: 600 };
const DELETED: ModerationContext = { type: 'deleted' };

const DELETED_CAPTIONS: Record<DeletedMessageStyle, string> = {
  strikethrough: 'crossed out and faded, with the reason in red',
  dimmed: 'faded, with no reason shown',
  italic: 'in muted italics, with a tag for what happened',
  keep: 'left exactly as it was sent',
  hidden: 'removed from chat',
};

/** Two removed messages (a timeout and a deletion) between ordinary ones. */
export const DeletedMessagePreview = ({
  design,
  style,
}: {
  design: PreviewDesign;
  style: DeletedMessageStyle;
}) => {
  // Keep leaves the row untouched, so it carries no moderation at all.
  const modStyle = style === 'keep' ? undefined : style;
  const hidden = style === 'hidden';
  return (
    <SettingPreview caption={DELETED_CAPTIONS[style]}>
      <PreviewChat design={design}>
        <Row>
          <PreviewLine design={design} who={PIXELFOX} text="that clutch was unreal" minutesAgo={3} first />
        </Row>
        <Row collapsed={hidden}>
          <PreviewLine
            design={design}
            who={LOOTGOBLIN}
            text="free subs at the link in my bio"
            minutesAgo={2}
            moderation={modStyle && TIMEOUT}
            deletedStyle={modStyle}
          />
        </Row>
        <Row>
          <PreviewLine design={design} who={MOSSY} text="gg, that boss had no chance" minutesAgo={2} />
        </Row>
        <Row collapsed={hidden}>
          <PreviewLine
            design={design}
            who={QUIETSTORM}
            text="the ending is a twist, she was the villain"
            minutesAgo={1}
            moderation={modStyle && DELETED}
            deletedStyle={modStyle}
          />
        </Row>
      </PreviewChat>
    </SettingPreview>
  );
};

/** The preview design from a stored chat design, for tabs other than Chat
 *  that show a sample (same fallbacks as ChatSettings' `cd`). */
const previewDesignOf = (stored: ChatDesignSettings | undefined): PreviewDesign => ({
  show_dividers: stored?.show_dividers ?? true,
  alternating_backgrounds: stored?.alternating_backgrounds ?? false,
  message_spacing: stored?.message_spacing ?? 8,
  font_size: stored?.font_size ?? 14,
  font_weight: stored?.font_weight ?? 400,
  show_timestamps: stored?.show_timestamps ?? false,
  show_timestamp_seconds: stored?.show_timestamp_seconds ?? false,
  timestamp_format: stored?.timestamp_format ?? '12h',
  username_separator: stored?.username_separator ?? (stored?.username_colon ? 'colon' : 'none'),
  username_style: stored?.username_style ?? 'plain',
  username_accent_source: stored?.username_accent_source ?? 'user',
});

/** Sample chat whose middle line has the real mod slider. Letting go only
 *  names the step; nothing is sent from a preview. */
export const ModSliderPreview = ({ stored }: { stored: ChatDesignSettings | undefined }) => {
  const design = previewDesignOf(stored);
  const [released, setReleased] = useState<string | null>(null);
  return (
    <SettingPreview
      caption={
        released
          ? `let go on ${released}; nothing is sent from the preview`
          : `drag the left edge of ${LOOTGOBLIN.name}'s message to the right`
      }
    >
      <PreviewChat design={design}>
        <Row>
          <PreviewLine design={design} who={PIXELFOX} text="that clutch was unreal" minutesAgo={3} first />
        </Row>
        <Row open>
          <div className="group relative isolate">
            <PreviewLine design={design} who={LOOTGOBLIN} text="free subs at the link in my bio" minutesAgo={2} />
            <ModSlider
              provider="twitch"
              target={{ targetName: LOOTGOBLIN.name }}
              messageId="preview"
              hint
              onApply={(step) => setReleased(step.label)}
            />
          </div>
        </Row>
        <Row>
          <PreviewLine design={design} who={MOSSY} text="gg, that boss had no chance" minutesAgo={1} />
        </Row>
      </PreviewChat>
    </SettingPreview>
  );
};

const LINE_SAMPLES: { who: Chatter; text: string }[] = [
  { who: PIXELFOX, text: 'that clutch was unreal' },
  { who: LOOTGOBLIN, text: 'free subs at the link in my bio' },
  { who: MOSSY, text: 'gg, that boss had no chance' },
];

/** Sample chat with the real line buttons: the same picture, the same
 *  pointer reading, the same On hover push and the same scroll over a
 *  timeout as chat. A click only names the action; nothing is sent from a
 *  preview. */
export const ModLinePreview = ({ stored }: { stored: ChatDesignSettings | undefined }) => {
  const design = previewDesignOf(stored);
  const steps = useModSliderSteps('twitch', useLineConfig());
  const sliderSteps = useModSliderSteps('twitch', useSliderConfig());
  const ladder = useMemo(() => sliderSteps?.filter((s) => s.action === 'timeout') ?? [], [sliderSteps]);
  const { buttons, nudge, reset, around } = useLineButtons(steps, ladder);
  const setButtons = useMemo(() => (steps && steps.length > 0 ? lineButtons(steps) : null), [steps]);
  const fit = useMemo(
    () => [...ladder, ...(steps ?? []).filter((s) => s.action === 'timeout')].map((s) => s.label),
    [ladder, steps],
  );
  const art = useLineArt(buttons, design.font_size, fit, setButtons);
  const show = stored?.mod_line_show === 'always' ? 'always' : 'hover';
  const [clicked, setClicked] = useState<string | null>(null);
  const hostRef = useRef<HTMLDivElement>(null);
  const artRef = useRef<{ art: LineArt; buttons: LineButton[] } | null>(null);
  const wheelRef = useRef({ nudge, reset, around });
  useLayoutEffect(() => {
    artRef.current = art && buttons ? { art, buttons } : null;
    wheelRef.current = { nudge, reset, around };
  });
  useEffect(() => {
    const el = hostRef.current;
    if (!el) return;
    const unbind = bindLineStrips(el, () => artRef.current, (hit) =>
      setClicked(`clicked ${hit.label} on ${hit.request.targetName ?? 'that chatter'}'s line`),
    );
    const unbindWheel = bindLineWheel(
      el,
      () => artRef.current,
      (index, dir) => wheelRef.current.nudge(index, dir),
      () => wheelRef.current.reset(),
      (index) => wheelRef.current.around(index),
    );
    return () => {
      unbind();
      unbindWheel();
    };
  }, []);
  return (
    <SettingPreview
      caption={
        clicked
          ? `${clicked}; nothing is sent from the preview`
          : show === 'hover'
            ? 'point at a line for its buttons; scroll over the timeout to change its length'
            : 'one click on a line\'s button acts on that chatter; scroll over the timeout to change its length'
      }
    >
      <div ref={hostRef} style={art?.vars} data-line-show={show}>
        <PreviewChat design={design}>
          {LINE_SAMPLES.map((s, i) => (
            <Row key={s.who.name}>
              <PreviewLine
                design={design}
                who={s.who}
                text={s.text}
                minutesAgo={LINE_SAMPLES.length - i}
                first={i === 0}
                lead={art && <ModLineButtons target={{ targetName: s.who.name }} messageId={`preview-${i}`} />}
              />
            </Row>
          ))}
        </PreviewChat>
      </div>
    </SettingPreview>
  );
};

/** Sample chat whose middle line shows the hover buttons as chat draws them
 *  under the pointer (here at rest, so they can be seen): the real Ban, Delete
 *  and quick timeout, the other lengths one click away, then Pin and Copy. A
 *  click only names the action; nothing is sent from a preview. */
export const ModHoverPreview = ({ stored }: { stored: ChatDesignSettings | undefined }) => {
  const design = previewDesignOf(stored);
  const [clicked, setClicked] = useState<string | null>(null);
  const [lengthsOpen, setLengthsOpen] = useState(false);
  const target = LOOTGOBLIN.name;
  const dockButton = 'p-1 rounded-md text-white/50 hover:text-white hover:bg-surface-hover transition-colors';
  return (
    <SettingPreview
      caption={
        clicked
          ? `${clicked}; nothing is sent from the preview`
          : `the buttons that show in the corner of the message under your pointer`
      }
    >
      <PreviewChat design={design}>
        <Row>
          <PreviewLine design={design} who={PIXELFOX} text="that clutch was unreal" minutesAgo={3} first />
        </Row>
        <Row>
          <div className="relative isolate bg-glass">
            <PreviewLine design={design} who={LOOTGOBLIN} text="free subs at the link in my bio" minutesAgo={2} />
            {/* The chat row's cluster (ChatMessage), shown at rest. */}
            <div
              className="absolute top-px right-2 z-[50] flex items-center gap-0.5 p-0.5 rounded-lg backdrop-blur-md border border-white/10 shadow-[0_4px_12px_rgba(0,0,0,0.4)]"
              style={{ backgroundColor: 'color-mix(in srgb, var(--color-background-tertiary) calc(95% + (1 - var(--glass-strength, 1)) * 5%), transparent)' }}
            >
              <HoverModButtons
                provider="twitch"
                targetName={target}
                canPunish
                canDelete
                lengthsOpen={lengthsOpen}
                onLengthsOpen={setLengthsOpen}
                onBan={(seconds) =>
                  setClicked(seconds == null ? `ban ${target}` : `time out ${target} for ${formatDuration(seconds)}`)
                }
                onDelete={() => setClicked(`delete ${target}'s message`)}
              />
              <div className="mx-0.5 h-4 w-px bg-white/10" />
              <button type="button" aria-label="Pin message" onClick={() => setClicked('pin the message')} className={dockButton}>
                <PushPin size={14} weight="bold" />
              </button>
              <button type="button" aria-label="Copy message" onClick={() => setClicked('copy the message')} className={dockButton}>
                <Copy size={14} weight="bold" />
              </button>
            </div>
          </div>
        </Row>
        <Row>
          <PreviewLine design={design} who={MOSSY} text="gg, that boss had no chance" minutesAgo={1} />
        </Row>
      </PreviewChat>
    </SettingPreview>
  );
};

/** Ordinary chat for the layout settings. The newest line replays its arrival
 *  whenever the arrival style changes, or on request. */
export const MessageLayoutPreview = ({
  design,
  entrance,
}: {
  design: PreviewDesign;
  entrance: string;
}) => {
  const [replay, setReplay] = useState(0);
  return (
    <SettingPreview caption="how messages sit in your chat" onReplay={() => setReplay((n) => n + 1)}>
      <PreviewChat design={design} entrance={entrance}>
        <Row>
          <PreviewLine design={design} who={PIXELFOX} text="that clutch was unreal" minutesAgo={3} first />
        </Row>
        <Row>
          <PreviewLine design={design} who={MOSSY} text="gg, that boss had no chance" minutesAgo={2} />
        </Row>
        <Row key={`${entrance}-${replay}`}>
          <PreviewLine design={design} who={QUIETSTORM} text="first time catching this live, hi chat" minutesAgo={0} />
        </Row>
      </PreviewChat>
    </SettingPreview>
  );
};

/** A gift sub between two chat lines, dressed as event rows are. The card
 *  remounts when its look changes so the glint plays again. */
export const EventRowPreview = ({
  design,
  events,
}: {
  design: PreviewDesign;
  events: ChatEventSettings;
}) => {
  const [replay, setReplay] = useState(0);
  const glint = events.event_animation && events.event_animation !== 'none';
  const pad = calculateHalfPadding(design.message_spacing);
  const adjust = useNameColorAdjust();
  const look = `${events.event_style ?? 'cards'}-${events.event_animation ?? 'none'}-${events.event_animate_repeat ? 1 : 0}-${events.event_outline_color ?? ''}`;
  return (
    <SettingPreview caption="a gift sub in your chat" onReplay={glint ? () => setReplay((n) => n + 1) : undefined}>
      <PreviewChat design={design}>
        <Row>
          <PreviewLine design={design} who={PIXELFOX} text="that clutch was unreal" minutesAgo={2} first />
        </Row>
        <div className="chat-message-row" key={`${look}-${replay}`}>
          {/* Event rows always carry their top rule, divider setting or not. */}
          <div
            className={`px-3 border-t border-borderSubtle ${eventCardClass(events, 'subscription-gradient')}`}
            style={{ ...eventCardStyle(events), paddingTop: `${pad}px`, paddingBottom: `${pad}px` }}
          >
            <div className="flex items-center gap-2.5">
              <Gift size={20} className="flex-shrink-0 text-highlight-purple" aria-hidden />
              <p
                className="min-w-0 flex-1 leading-relaxed font-semibold text-white"
                style={{ fontSize: `${design.font_size}px` }}
              >
                <span style={{ color: adjust(MOSSY.color) ?? MOSSY.color }}>{MOSSY.name}</span> gifted a
                subscription to <span style={{ color: adjust(QUIETSTORM.color) ?? QUIETSTORM.color }}>{QUIETSTORM.name}</span>!
              </p>
            </div>
          </div>
        </div>
        <Row>
          <PreviewLine design={design} who={LOOTGOBLIN} text="welcome to the club" minutesAgo={1} />
        </Row>
      </PreviewChat>
    </SettingPreview>
  );
};

/** A mention drawn the way chat draws it (chat/mentionStyle), in a plain line
 *  and inside a /me line, where the slant setting shows. */
export const MentionPreview = ({ design, look }: { design: PreviewDesign; look: MentionLook }) => {
  const adjust = useNameColorAdjust();
  const mention = (who: Chatter) => {
    const c = adjust(who.color) ?? who.color;
    return (
      <span className={mentionBoxClass(look)} style={mentionBoxStyle(look, c)}>
        <span style={mentionTextStyle(look, { color: c })}>@{who.name}</span>
      </span>
    );
  };
  return (
    <SettingPreview caption="a mention, and one inside a /me message">
      <PreviewChat design={design}>
        <Row>
          <PreviewLine
            design={design}
            who={PIXELFOX}
            text={<>{mention(MOSSY)} that clutch was unreal</>}
            minutesAgo={2}
            first
          />
        </Row>
        <Row>
          <PreviewLine
            design={design}
            who={LOOTGOBLIN}
            text={<>high-fives {mention(QUIETSTORM)}</>}
            minutesAgo={1}
            action
          />
        </Row>
      </PreviewChat>
    </SettingPreview>
  );
};

interface MarkSample {
  who: Chatter;
  text: string;
  mark?: ScrollbarMarkKind;
  color?: string;
}

// A stretch of chat with one of each mark kind, scrolled to its newest lines
// so the marked ones sit above, out of sight, as they do when you read back.
const MARK_SAMPLES: MarkSample[] = [
  { who: MOSSY, text: 'evening chat' },
  { who: PIXELFOX, text: '@you did you catch that play?', mark: 'mention' },
  { who: LOOTGOBLIN, text: 'subscribed for 6 months!', mark: 'sub' },
  { who: QUIETSTORM, text: 'lol' },
  { who: MOSSY, text: 'Drops are live, link in the panels', mark: 'announcement', color: '#22c55e' },
  { who: PIXELFOX, text: 'that clutch though', mark: 'highlight' },
  { who: LOOTGOBLIN, text: 'GG' },
  { who: QUIETSTORM, text: 'schedule is in the about tab', mark: 'pinned' },
  { who: MOSSY, text: 'is raiding with 120 viewers', mark: 'raid' },
  { who: PIXELFOX, text: 'hydrate check' },
  { who: LOOTGOBLIN, text: 'first time here, hi!', mark: 'first_time', color: '#a855f7' },
  { who: QUIETSTORM, text: 'redeemed Hydrate', mark: 'redeem' },
  { who: MOSSY, text: 'back to the queue' },
  { who: PIXELFOX, text: 'one more game' },
  { who: LOOTGOBLIN, text: 'he is so locked in' },
  { who: QUIETSTORM, text: 'POG' },
];

const sampleId = (i: number) => `mark-sample-${i}`;

/** Sample chat read back from the bottom, with the real scrollbar marks drawn
 *  over its scrollbar for the kinds that are on. */
export const ScrollbarMarksPreview = ({
  design,
  kinds,
  mentionColor,
}: {
  design: PreviewDesign;
  kinds: readonly string[];
  mentionColor?: string;
}) => {
  const scrollerRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const marks = useMemo(() => {
    const rows = MARK_SAMPLES.map(
      (s, i) =>
        ({
          id: sampleId(i),
          metadata:
            s.mark && s.mark !== 'pinned'
              ? { scrollbar_mark: { kind: s.mark, color: s.mark === 'mention' ? mentionColor : s.color } }
              : {},
        }) as unknown as BackendChatMessage,
    );
    const pinned = new Set(MARK_SAMPLES.flatMap((s, i) => (s.mark === 'pinned' ? [sampleId(i)] : [])));
    return collectMarks(rows, (m) => (typeof m === 'string' ? null : m.id), new Set(kinds), pinned);
  }, [kinds, mentionColor]);

  useLayoutEffect(() => {
    const el = scrollerRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [design.font_size, design.message_spacing, design.show_timestamps]);

  return (
    <SettingPreview caption={kinds.length ? 'ticks mark what is above, while you read back' : 'no kinds are on'}>
      <div className="glass-panel relative overflow-hidden rounded-lg">
        <div className="relative h-36">
          <div ref={scrollerRef} className="h-full overflow-y-auto overflow-x-hidden scrollbar-thin">
            <div ref={contentRef} className={`flex flex-col py-1${design.alternating_backgrounds ? ' chat-striped' : ''}`}>
              {MARK_SAMPLES.map((s, i) => (
                <div key={sampleId(i)} data-message-id={sampleId(i)} className="chat-message-row">
                  <PreviewLine design={design} who={s.who} text={s.text} minutesAgo={MARK_SAMPLES.length - i} first={i === 0} />
                </div>
              ))}
            </div>
          </div>
          <ScrollbarMarks scrollerRef={scrollerRef} contentRef={contentRef} marks={marks} />
        </div>
      </div>
    </SettingPreview>
  );
};

/** Followers-only (10 minutes) and slow mode (30 s), in the words Rust ships
 *  for them (services/room_modes.rs, several_modes_list_most_limiting_first). */
const SAMPLE_ROOM_MODES: RoomModeView = {
  modes: [
    {
      kind: 'followers_only',
      label: 'Followers-only chat (10 minutes)',
      compact: 'Followers-only (10m)',
      short: 'Followers 10m',
      detail: 'Only accounts that have followed for 10 minutes can chat.',
      applies_to_you: null,
    },
    {
      kind: 'slow',
      label: 'Slow mode (30 seconds)',
      compact: 'Slow mode (30s)',
      short: 'Slow 30s',
      detail: 'Chatters wait 30 seconds between messages.',
      applies_to_you: true,
    },
  ],
  chip: 'Followers 10m',
  exempt: false,
  prompt: 'You can chat in 4 minutes',
  prompt_short: 'Chat in 4m',
  standing: null,
  slow_seconds: 30,
};

const ROOM_MODE_CAPTIONS: Record<'chip' | 'prompt', string> = {
  chip: 'a followers-only room in slow mode; hover the chip for both',
  prompt: 'a 10-minute followers-only room you followed 6 minutes ago',
};

/** The message box in a room with chat modes on, drawn the chosen way: the
 *  real chip, or the prompt in place of "Send a message". */
export const RoomModesPreview = ({ style }: { style: 'chip' | 'prompt' }) => (
  <SettingPreview caption={ROOM_MODE_CAPTIONS[style]}>
    <div className="sn-composer-field relative flex items-center">
      <div
        className="glass-input flex h-9 w-full items-center rounded-lg pl-3 text-sm text-textSecondary"
        style={{ paddingRight: 'var(--sn-room-pad, 12px)' }}
      >
        <span className="truncate">{style === 'prompt' ? SAMPLE_ROOM_MODES.prompt : 'Send a message'}</span>
      </div>
      {style === 'chip' && <RoomModeIndicator view={SAMPLE_ROOM_MODES} />}
    </div>
  </SettingPreview>
);

/** The phones' message bar with the inbox tray at the chosen end and the chat
 *  menu at the other, drawn as the composer draws them; nothing in it acts. */
export const InboxButtonPreview = ({ side }: { side: 'left' | 'right' }) => {
  const tray = (
    <span className="chrome-glaze chrome-glaze--control relative shrink-0 flex items-center justify-center w-10 h-10 text-textSecondary">
      <Inbox size={19} strokeWidth={2} />
      <span className={`sn-inbox-badge absolute -top-1 ${side === 'right' ? '-left-1' : '-right-1'} min-w-[17px] h-[17px] px-[5px] rounded-full text-[10.5px] font-bold leading-[17px] text-center tabular-nums`}>
        2
      </span>
    </span>
  );
  const menu = (
    <span className="chrome-glaze chrome-glaze--control shrink-0 flex items-center justify-center w-10 h-10 text-textSecondary">
      <DotsThreeVertical size={22} weight="bold" />
    </span>
  );
  return (
    <SettingPreview caption={side === 'right' ? 'the inbox under a right thumb' : 'the inbox on the left, the chat menu on the right'}>
      <div className="flex items-center gap-1.5" aria-hidden>
        {side === 'left' ? tray : menu}
        <span className="glass-input flex-1 min-w-0 h-10 flex items-center px-3 text-[15px] text-textMuted">
          <span className="truncate">Send a message</span>
        </span>
        {side === 'left' ? menu : tray}
      </div>
    </SettingPreview>
  );
};

/** A mention and a reply to you as they sit in chat after landing: the color
 *  edge, and with Keep tinted the held wash chat rows draw (chat/mentionHold). */
export const MentionRowPreview = ({
  design,
  hold,
  opacity,
  mentionColor,
  replyColor,
}: {
  design: PreviewDesign;
  hold: boolean;
  opacity: number;
  mentionColor: string;
  replyColor: string;
}) => {
  const rows: { who: Chatter; text: string; color: string }[] = [
    { who: PIXELFOX, text: '@you that clutch was unreal', color: mentionColor },
    { who: MOSSY, text: 'right? best play of the night', color: replyColor },
  ];
  return (
    <SettingPreview caption={hold ? 'a mention and a reply to you, tinted the whole time' : 'a mention and a reply to you after the flash'}>
      <PreviewChat design={design}>
        {rows.map((r, i) => (
          <Row key={i}>
            <div
              style={{
                borderLeft: `4px solid ${r.color}`,
                backgroundImage: hold ? mentionHoldBackground(r.color, opacity) : undefined,
              }}
            >
              <PreviewLine design={design} who={r.who} text={r.text} minutesAgo={2 - i} first={i === 0} />
            </div>
          </Row>
        ))}
      </PreviewChat>
    </SettingPreview>
  );
};
