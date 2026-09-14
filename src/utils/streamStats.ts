// Stream stat formatters shared by the sidebar hover card, the chat header and
// the Compact View title-bar chips, so all three read the same.

/** 1234567 -> "1.2M", 12300 -> "12.3K", 999 -> "999". */
export function formatViewerCount(count: number): string {
    if (count >= 1000000) {
        return (count / 1000000).toFixed(1) + 'M';
    } else if (count >= 1000) {
        return (count / 1000).toFixed(1) + 'K';
    }
    return count.toString();
}

/**
 * A ticking stream clock: "H:MM:SS" once past the first hour, "M:SS" before it.
 * Returns '' when there is no usable start time, and clamps a start time in the
 * future to "0:00" rather than counting backwards on a skewed clock.
 * `now` is injectable so the formatter can be tested without the wall clock.
 */
export function formatUptimeClock(
    startedAt: string | null | undefined,
    now: number = Date.now(),
): string {
    if (!startedAt) return '';
    const start = Date.parse(startedAt);
    if (!Number.isFinite(start)) return '';

    const diffMs = now - start;
    if (diffMs <= 0) return '0:00';

    const hours = Math.floor(diffMs / (1000 * 60 * 60));
    const minutes = Math.floor((diffMs % (1000 * 60 * 60)) / (1000 * 60));
    const seconds = Math.floor((diffMs % (1000 * 60)) / 1000);

    return hours > 0
        ? `${hours}:${minutes.toString().padStart(2, '0')}:${seconds.toString().padStart(2, '0')}`
        : `${minutes}:${seconds.toString().padStart(2, '0')}`;
}
