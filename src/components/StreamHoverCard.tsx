import { Gift } from 'lucide-react';
import StreamTitleWithEmojis from './StreamTitleWithEmojis';
import { formatStreamUptime } from '../utils/chatCommands';
import type { TwitchStream } from '../types';

// Replaces the tooltip's default centered pill. Padding lives on the content
// block instead, so the preview can run edge to edge under the rounded corners.
export const STREAM_HOVER_CARD_CLASS =
    'w-72 overflow-hidden rounded-lg bg-black/85 text-left shadow-xl backdrop-blur-xl border border-white/10 pointer-events-none';

// Helix hands back the placeholder in two spellings depending on the endpoint.
// 440x248 is roughly the card's width at 1.5x, so it stays sharp without
// pulling the 1280x720 the stream cards use.
const thumbnailUrl = (url: string) =>
    url
        .replace('%{width}', '440').replace('%{height}', '248')
        .replace('{width}', '440').replace('{height}', '248');

interface StreamHoverCardProps {
    stream: TwitchStream;
    hasDrops?: boolean;
}

/**
 * What a sidebar row shows on hover: the answer to "what is this person
 * streaming right now" without clicking in and waiting for the player to load.
 *
 * Deliberately built from the same parts as a Home stream card (preview,
 * `live-dot`, `glass-badge`) so the two read as one family. The preview also
 * does the layout work a wall of text could not: it gives the card a focal
 * point and takes the live numbers off the text block, which leaves a clean
 * who / what / where stack underneath.
 *
 * Stateless on purpose. It only mounts while the tooltip is open, so everything
 * computed here is already current as of the hover, and a card that lives for a
 * few seconds has no use for a ticking clock.
 */
const StreamHoverCard = ({ stream, hasDrops }: StreamHoverCardProps) => {
    const uptime = formatStreamUptime(stream.started_at);

    return (
        <div>
            {/* Preview. aspect-video reserves the height before the image
                lands, so the tooltip measures the card correctly on first
                paint instead of shifting once it loads. */}
            <div className="relative w-full aspect-video overflow-hidden bg-white/5">
                {stream.thumbnail_url && (
                    <img
                        src={thumbnailUrl(stream.thumbnail_url)}
                        alt=""
                        draggable={false}
                        className="w-full h-full object-cover"
                    />
                )}

                <div className="absolute top-1.5 left-1.5 flex items-center gap-1">
                    <div className="live-dot text-xs px-1.5 py-0.5">LIVE</div>
                    {hasDrops && (
                        <div className="drops-badge-glass">
                            <Gift size={10} />
                            <span>DROPS</span>
                        </div>
                    )}
                </div>

                <div className="absolute bottom-1.5 left-1.5 px-2 py-0.5 glass-badge rounded text-white text-[10px] font-medium tabular-nums">
                    {stream.viewer_count.toLocaleString()} viewers
                </div>
                {uptime && (
                    <div className="absolute bottom-1.5 right-1.5 px-2 py-0.5 glass-badge rounded text-white text-[10px] font-medium tabular-nums">
                        {uptime}
                    </div>
                )}
            </div>

            <div className="px-3 py-2.5">
                <div className="flex items-center gap-1 min-w-0">
                    <span className="text-[13px] font-semibold text-textPrimary truncate">
                        {stream.user_name || stream.user_login}
                    </span>
                    {stream.broadcaster_type === 'partner' && (
                        <svg className="w-3 h-3 flex-shrink-0" viewBox="0 0 16 16" fill="#9146FF">
                            <path fillRule="evenodd" d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z" clipRule="evenodd" />
                        </svg>
                    )}
                </div>

                {/* Titles are emoji-heavy. The shared renderer sizes them for
                    body copy, which at this text size makes them tower over the
                    channel name, so bring them back down to the line. */}
                {stream.title?.trim() && (
                    <div className="mt-0.5 text-[11.5px] leading-snug text-white/65 line-clamp-2 [&_img]:w-3.5 [&_img]:h-3.5">
                        <StreamTitleWithEmojis title={stream.title} />
                    </div>
                )}

                <div className="mt-1 text-[11px] text-textMuted truncate">
                    {stream.game_name || 'Just Chatting'}
                </div>
            </div>
        </div>
    );
};

export default StreamHoverCard;
