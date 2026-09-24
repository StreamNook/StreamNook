import { useEffect, useRef } from 'react';
import { Timer, UsersThree } from 'phosphor-react';
import { useAppStore } from '../stores/AppStore';
import { unwatchChannel, useChannelState, watchChannel } from '../stores/channelStateStore';
import { formatUptimeClock } from '../utils/streamStats';
import { streamProvider } from '../utils/streamProvider';
import { TogetherChip } from './SharedViewers';

/**
 * Viewer count and stream uptime for Compact View. Compact View sets
 * chatPlacement to 'hidden', which unmounts ChatWidget entirely, and with it
 * the chat header's watch on the channel and the ticker that normally live
 * there. This holds its own watch on Rust's channel state so the numbers,
 * Shared Viewership included, stay visible without opening chat.
 *
 * Self-contained on purpose: the title bar must not re-render once a second.
 */
const CompactStreamStats = () => {
    const currentStream = useAppStore((s) => s.currentStream);
    const uptimeRef = useRef<HTMLSpanElement | null>(null);

    // Channel state is keyed by Twitch login. A Kick or YouTube slug that
    // happens to match a Twitch account would read that stranger's count, so
    // other providers keep the number the store carries.
    const userLogin = streamProvider(currentStream) === 'twitch' ? currentStream?.user_login?.toLowerCase() : undefined;
    const userId = currentStream?.user_id;
    const startedAt = currentStream?.started_at;
    const channelState = useChannelState(userLogin);

    useEffect(() => {
        if (!userLogin || !userId) return;
        void watchChannel(userLogin, userId);
        return () => {
            void unwatchChannel(userLogin);
        };
    }, [userLogin, userId]);

    // Until Rust's first answer lands, show the count the store captured when
    // the stream started, so entering Compact View never flashes an empty slot.
    const viewerCount = channelState?.viewers_at != null
        ? channelState.viewer_count
        : currentStream?.viewer_count ?? null;
    const collab = userLogin ? (channelState?.collab ?? null) : null;

    // Write the clock straight into the span rather than through state, so a
    // 1 Hz tick never re-renders the title bar. Uses its own ref instead of the
    // chat header's `stream-uptime-display` id, which stays that header's.
    useEffect(() => {
        const tick = () => {
            if (uptimeRef.current) uptimeRef.current.textContent = formatUptimeClock(startedAt);
        };
        tick();
        const id = setInterval(tick, 1000);
        return () => clearInterval(id);
    }, [startedAt]);

    if (!currentStream) return null;

    return (
        <>
            {viewerCount !== null && collab && (
                <TogetherChip
                    variant="header"
                    collab={collab}
                    onOpenChannel={(login) => void useAppStore.getState().startStream(login)}
                    allowMultiNook
                />
            )}
            {viewerCount !== null && (
                <div className="flex items-center gap-1 text-xs text-textSecondary">
                    <UsersThree size={13} weight="fill" />
                    <span className="tabular-nums">{viewerCount.toLocaleString()}</span>
                </div>
            )}
            {startedAt && (
                <div className="flex items-center gap-1 text-xs text-textSecondary">
                    <Timer size={13} weight="bold" />
                    <span ref={uptimeRef} className="tabular-nums" />
                </div>
            )}
        </>
    );
};

export default CompactStreamStats;
