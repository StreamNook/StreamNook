// The card over the composer while a reply is being written: who you are
// replying to, drawn the way their chat row draws them (badges, then the name
// in its 7TV paint or color, through the same chatter look the pinned message
// uses), and one line of what they said, emotes included. A glazed banner
// like the other banners above the message box.
import { motion } from 'framer-motion';
import { ArrowBendUpLeft, X } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { usePerformanceQuiet } from '../../utils/lowPower';
import type { BackendChatMessage, MessageSegment } from '../../services/twitchChat';
import type { ProviderId } from '../../types/providers';
import { ProviderMark } from '../ProviderLogo';
import { Tooltip } from '../ui/Tooltip';
import { COMPOSER_STRIP, COMPOSER_STRIP_STYLE } from '../ui/glazeButtons';
import { PinnedChatter } from './PinnedChatter';

export interface ReplyContextProps {
  username: string;
  /** The message being answered, when chat still holds it. */
  row: BackendChatMessage | null;
  /** Set when the reply goes to another platform's room in a merged feed. */
  source?: { provider: ProviderId; channelName: string };
  onCancel: () => void;
}

const EASE = [0.2, 0.8, 0.2, 1] as const;

// A "/me" line arrives wrapped as SOH "ACTION " ... SOH; quote just its words.
const SOH = String.fromCharCode(1);
const actionText = (content: string) =>
  content.startsWith(`${SOH}ACTION `) && content.endsWith(SOH) ? content.slice(8, -1) : content;

/** The quoted message on one line: words as text, emotes and emoji as small
 *  pictures, everything else by its text. */
function Quote({ segments, content }: { segments?: MessageSegment[]; content: string }) {
  if (!segments || segments.length === 0) return <>{content}</>;
  return (
    <>
      {segments.map((s, i) =>
        s.type === 'emote' || s.type === 'emoji' ? (
          <img
            key={i}
            src={s.type === 'emote' ? s.local_path || s.emote_url : s.emoji_url}
            alt={s.content}
            draggable={false}
            className="mx-px inline-block h-[1.35em] w-auto align-middle"
          />
        ) : (
          <span key={i}>{s.content}</span>
        ),
      )}
    </>
  );
}

export function ReplyContextCard({ username, row, source, onCancel }: ReplyContextProps) {
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
  const performanceQuiet = usePerformanceQuiet();
  const instant = motionOff || performanceQuiet;
  const provider: ProviderId = row?.provider ?? 'twitch';
  const userId = row ? row.tags?.['user-id'] || row.user_id || '' : '';
  const channelId = row?.tags?.['source-room-id'] || row?.tags?.['room-id'];
  const badges = row?.badges.map((b) => ({ set_id: b.name, version: b.version }));
  const quote = row ? actionText(row.content).trim() : '';
  return (
    <motion.div
      initial={{ opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0, transition: instant ? { duration: 0 } : { duration: 0.14, ease: EASE } }}
      data-reply-card=""
      className={COMPOSER_STRIP}
      style={COMPOSER_STRIP_STYLE}
    >
      <ArrowBendUpLeft size={16} weight="bold" aria-hidden className="shrink-0 text-accent" />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1.5 text-[12.5px] leading-[18px]">
          <span className="shrink-0 text-textSecondary">Replying to</span>
          {row ? (
            <PinnedChatter
              provider={provider}
              userId={userId}
              name={row.display_name || row.username}
              login={row.username}
              shownName={row.metadata?.shown_name}
              color={row.color || row.tags?.color}
              badges={badges}
              channelId={channelId}
              variant="line"
              nameClassName="min-w-0 truncate font-bold"
            />
          ) : (
            <span className="min-w-0 truncate font-bold text-accent">{username}</span>
          )}
          {/* Where it goes, the moment the reply is aimed somewhere other
              than the channel on screen, rather than a surprise after sending. */}
          {source && (
            <span className="inline-flex shrink-0 items-center gap-1 text-textSecondary">
              on
              <ProviderMark provider={source.provider} size={12} />
              <span className="text-textPrimary">{source.channelName}</span>
            </span>
          )}
        </div>
        {quote && (
          <div className="mt-0.5 truncate text-[12px] leading-[18px] text-textSecondary">
            <Quote segments={row?.segments} content={quote} />
          </div>
        )}
      </div>
      <Tooltip content="Cancel reply" side="top">
        <button
          type="button"
          aria-label="Cancel reply"
          onClick={onCancel}
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-textSecondary transition-colors hover:bg-white/[0.08] hover:text-textPrimary"
        >
          <X size={13} weight="bold" />
        </button>
      </Tooltip>
    </motion.div>
  );
}
