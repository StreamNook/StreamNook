import { Logger } from './logger';

// Display helpers for account facts that come from IVR. The facts themselves are
// fetched and cached in Rust (services/ivr.rs, get_ivr_*_summary).

/**
 * Formats a date string into a human-readable format
 * @param dateString - ISO date string
 * @param includeRelative - Whether to include relative time (e.g., "5 years ago")
 * @returns Formatted date string
 */
export function formatIVRDate(dateString: string, includeRelative: boolean = true): string {
    try {
        const date = new Date(dateString);
        const now = new Date();

        // Format the absolute date
        const absoluteDate = date.toLocaleDateString('en-US', {
            year: 'numeric',
            month: 'long',
            day: 'numeric'
        });

        if (!includeRelative) {
            return absoluteDate;
        }

        // Calculate relative time
        const diffMs = now.getTime() - date.getTime();
        const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));
        const diffMonths = Math.floor(diffDays / 30);
        const diffYears = Math.floor(diffDays / 365);

        let relativeTime: string;
        if (diffYears > 0) {
            relativeTime = diffYears === 1 ? '1 year ago' : `${diffYears} years ago`;
        } else if (diffMonths > 0) {
            relativeTime = diffMonths === 1 ? '1 month ago' : `${diffMonths} months ago`;
        } else if (diffDays > 0) {
            relativeTime = diffDays === 1 ? '1 day ago' : `${diffDays} days ago`;
        } else {
            relativeTime = 'today';
        }

        return `${absoluteDate} (${relativeTime})`;
    } catch (error) {
        Logger.error('[IVR] Failed to format date:', error);
        return dateString;
    }
}

/**
 * Formats subscription tenure with streak and cumulative months
 * @param streak - Current streak months
 * @param cumulative - Total cumulative months
 * @returns Formatted tenure string
 */
export function formatSubTenure(streak: number | null, cumulative: number | null): string {
    if (streak === null && cumulative === null) return '';
    if (streak === null) return `${cumulative} months`;
    if (cumulative === null) return `${streak} months`;

    if (streak === cumulative) {
        return `${streak} ${streak === 1 ? 'month' : 'months'}`;
    }

    return `${streak} ${streak === 1 ? 'month' : 'months'} (${cumulative} cumulative)`;
}
