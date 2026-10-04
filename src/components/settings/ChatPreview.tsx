import { useState, type CSSProperties, type ReactNode } from 'react';
import { Gift, RotateCcw } from 'lucide-react';
import { useNameColorAdjust } from '../../hooks/useNameColor';
import { StyledChatName, type NameSeparator, type NameStyle } from '../chat/StyledChatName';
import { deletedBodyStyle, deletedRowDimmed, type DeletedMessageStyle } from '../chat/deletedMessage';
import { ModerationTag } from '../chat/ModerationTag';
import { eventCardClass, eventCardStyle } from '../chat/eventCard';
import type { ModerationContext } from '../../stores/chatConnectionStore';
import type { ChatEventSettings } from '../../types';
import { Tooltip } from '../ui/Tooltip';
import { calculateHalfPadding } from '../../utils/chatLayoutUtils';

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
}: {
  design: PreviewDesign;
  who: Chatter;
  text: string;
  minutesAgo: number;
  first?: boolean;
  moderation?: ModerationContext;
  deletedStyle?: DeletedMessageStyle;
}) => {
  const adjust = useNameColorAdjust();
  const color = adjust(who.color) ?? who.color;
  const box = rowBox(design, first);
  const dim = moderation && deletedStyle && deletedRowDimmed(deletedStyle);
  return (
    <div
      className={`${box.className} transition-opacity duration-200`}
      style={{ ...box.style, opacity: dim ? 0.5 : undefined }}
    >
      {design.show_timestamps && (
        <div className="mb-0.5 text-[10px] leading-tight text-textSecondary opacity-50">
          {sampleTime(design, minutesAgo)}
        </div>
      )}
      <span
        className="leading-relaxed align-middle"
        style={{ fontSize: `${design.font_size}px`, fontWeight: design.font_weight }}
      >
        <StyledChatName
          name={who.name}
          nameTextStyle={{ color }}
          nameStyle={design.username_style}
          separator={design.username_separator}
          accentColor={design.username_accent_source === 'theme' ? 'var(--color-accent)' : color}
        />
        <span style={{ fontWeight: 'var(--chat-body-weight, 300)' }} className="text-textPrimary break-words">
          <span style={moderation && deletedStyle ? deletedBodyStyle(deletedStyle) : undefined}>
            {' '}
            {text}
          </span>
          {moderation && deletedStyle && <ModerationTag context={moderation} style={deletedStyle} />}
        </span>
      </span>
    </div>
  );
};

/** A plain chat line as a list child (what striping and arrival key off). */
const Row = ({ children, collapsed = false }: { children: ReactNode; collapsed?: boolean }) => (
  <div
    className="chat-message-row grid transition-[grid-template-rows,opacity] duration-300 ease-out"
    style={{ gridTemplateRows: collapsed ? '0fr' : '1fr', opacity: collapsed ? 0 : 1 }}
    aria-hidden={collapsed || undefined}
  >
    <div className="min-h-0 overflow-hidden">{children}</div>
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
