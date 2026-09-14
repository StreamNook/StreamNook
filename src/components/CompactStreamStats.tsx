import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Clock, Eye } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import type { TwitchStream } from '../types';
import { formatUptimeClock } from '../utils/streamStats';
import { streamProvider } from '../utils/streamProvider';
import { useVisibleInterval } from '../utils/useVisibleInterval';
import { Logger } from '../utils/logger';

const VIEWER_POLL_MS = 60000;

/**
 * Viewer count and stream uptime for Compact View. Compact View sets
 * chatPlacement to 'hidden', which unmounts ChatWidget entirely, so the poll and
 * the ticker that normally live in the chat header are not even running there.
 * This owns its own copies so the numbers stay visible without opening chat.
 *
 * Self-contained on purpose: the title bar must not re-render once a second.
 */
const CompactStreamStats = () => {
    const currentStream = useAppStore((s) => s.currentStream);
    // Tagged with the channel it was fetched for, so switching streams shows a
    // blank rather than the previous channel's number, without needing an
    // effect that resets state on change.
    const [fetched, setFetched] = useState<{ login: string; count: number | null } | null>(null);
    const uptimeRef = useRef<HTMLSpanElement | null>(null);

    // Helix answers for Twitch logins only. A Kick or YouTube slug that happens
    // to match a Twitch account would read that stranger's count, so other
    // providers keep the number the store carries and never poll here.
    const userLogin = streamProvider(currentStream) === 'twitch' ? currentStream?.user_login : undefined;
    const startedAt = currentStream?.started_at;
    // Until the first poll lands, show the count the store captured when the
    // stream started, so entering Compact View never flashes an empty slot.
    const viewerCount = fetched && fetched.login === userLogin
        ? fetched.count
        : currentStream?.viewer_count ?? null;

    const getViewerCount = useCallback(async () => {
        if (!userLogin) return;
        try {
            // Rust makes the Helix call; an offline channel answers null.
            const live = await invoke<TwitchStream | null>('check_stream_online', { userLogin });
            setFetched({ login: userLogin, count: live?.viewer_count ?? null });
        } catch (err) {
            Logger.error('[CompactStreamStats] Failed to fetch viewer count:', err);
            setFetched({ login: userLogin, count: null });
        }
    }, [userLogin]);

    // Prime the count on mount; useVisibleInterval only fires on its own
    // schedule, so without this the seed value would sit there for a minute.
    // Kicked off from a timer rather than straight from the effect body so the
    // fetch starts after the commit instead of during it.
    useEffect(() => {
        const id = setTimeout(() => void getViewerCount(), 0);
        return () => clearTimeout(id);
    }, [getViewerCount]);
    useVisibleInterval(getViewerCount, VIEWER_POLL_MS);

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
            {viewerCount !== null && (
                <div className="flex items-center gap-1 text-xs text-textSecondary">
                    <Eye size={13} />
                    <span className="tabular-nums">{viewerCount.toLocaleString()}</span>
                </div>
            )}
            {startedAt && (
                <div className="flex items-center gap-1 text-xs text-textSecondary">
                    <Clock size={13} />
                    <span ref={uptimeRef} className="tabular-nums" />
                </div>
            )}
        </>
    );
};

export default CompactStreamStats;
