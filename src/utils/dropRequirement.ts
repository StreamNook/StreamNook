// Words for drops that need more than one sitting of watch time or a
// subscription. Rust (services/reward_drops.rs) reads Twitch's numbers into
// the drop model; this only turns them into short phrases for every drops
// surface (Drops center, phone, title bar) to share.
import type { DropProgress, TimeBasedDrop } from '../types';

type RequirementFields = Pick<TimeBasedDrop, 'required_minutes_watched' | 'required_subs' | 'required_days' | 'random_of'>;

/** A container that gives one reward from a pool (a Great Ball) is opened, not claimed. */
function isDraw(drop: RequirementFields, progress?: DropProgress | null): boolean {
    return (drop.random_of ?? 0) > 1 || progress?.twitch_progress?.earned?.distribution_type === 'POOL';
}

/**
 * A tier that only gets you a container (subscribe for a Special Great Ball).
 * The ball itself is shown by its unlock tier, so this is how you get it, not
 * a reward of its own: reward and collection lists leave it out.
 */
export function obtainsContainer(drop: RequirementFields): boolean {
    return (drop.required_subs ?? 0) > 0 && (drop.random_of ?? 0) > 1;
}

/** What the claim button says: "Open" for a ball, "Claim" for anything else. */
export function claimLabel(drop: RequirementFields, progress?: DropProgress | null): string {
    return isDraw(drop, progress) ? 'Open' : 'Claim';
}

function minutesLabel(minutes: number): string {
    if (minutes >= 60 && minutes % 60 === 0) return `${minutes / 60} hr`;
    return `${Math.round(minutes)} min`;
}

/**
 * "Subscribe or gift a sub", "2 subs or gift subs", "20 min a day · 3 days".
 * Null for a plain watch-time drop, which the minutes already describe.
 */
export function dropRequirementText(drop: RequirementFields): string | null {
    const subs = drop.required_subs ?? 0;
    if (subs > 0) return subs === 1 ? 'Subscribe or gift a sub' : `${subs} subs or gift subs`;
    const days = drop.required_days ?? 0;
    if (days > 1 && drop.required_minutes_watched > 0) {
        return `${minutesLabel(drop.required_minutes_watched / days)} a day · ${days} days`;
    }
    return null;
}

/** "in 23h", "in 40m", "soon". */
export function resetsInLabel(iso: string | null | undefined, now: number = Date.now()): string {
    if (!iso) return '';
    const left = new Date(iso).getTime() - now;
    if (!Number.isFinite(left) || left <= 60_000) return 'soon';
    const hours = Math.floor(left / 3_600_000);
    if (hours >= 1) return `in ${hours}h`;
    return `in ${Math.ceil(left / 60_000)}m`;
}

/**
 * Twitch's progress on a multi-day or subscription drop as one line:
 * "You got Bulbasaur", "Ready to open", "1 of 2 subs",
 * "Day 2 of 3 · 5/20 min today · resets in 23h". Null when Twitch reported
 * nothing beyond minutes.
 */
export function twitchProgressLine(
    progress: DropProgress | null | undefined,
    drop: RequirementFields,
    now: number = Date.now(),
): string | null {
    const t = progress?.twitch_progress;
    if (!progress || !t) return null;
    if (progress.is_claimed) return t.earned ? `You got ${t.earned.name}` : 'Claimed';
    if (t.ready_to_claim) {
        if (isDraw(drop, progress)) return 'Ready to open';
        return t.earned ? `${t.earned.name} ready to claim` : 'Ready to claim';
    }
    const subs = drop.required_subs ?? 0;
    if (subs > 0) return `${Math.min(t.subs_done, subs)} of ${subs} ${subs === 1 ? 'sub' : 'subs'}`;
    const days = drop.required_days ?? 0;
    if (days > 1 && drop.required_minutes_watched > 0) {
        const perDay = Math.round(drop.required_minutes_watched / days);
        // Twitch counts today in days_done as soon as today's minutes are in
        // ("20/20m (Day 2)" at days_done 2), and the next day opens when this
        // window closes.
        const todayDone = t.minutes_today >= perDay;
        const day = Math.min(Math.max(todayDone ? t.days_done : t.days_done + 1, 1), days);
        const parts = [`Day ${day} of ${days}`];
        const opens = resetsInLabel(t.window_expires_at, now);
        if (todayDone) {
            parts.push('done for today');
            if (t.window_expires_at && opens && day < days) parts.push(`next day ${opens}`);
        } else {
            parts.push(`${t.minutes_today}/${perDay} min today`);
            if (t.window_expires_at && opens) parts.push(`resets ${opens}`);
        }
        return parts.join(' · ');
    }
    return null;
}

/** Completion 0-100 of one drop, from its own progress. */
export function dropPercent(progress: DropProgress | null | undefined, drop: RequirementFields): number {
    if (!progress) return 0;
    if (progress.is_claimed || progress.twitch_progress?.ready_to_claim) return 100;
    const subs = drop.required_subs ?? 0;
    if (subs > 0) return Math.min(100, ((progress.twitch_progress?.subs_done ?? 0) / subs) * 100);
    const required = progress.required_minutes_watched || drop.required_minutes_watched;
    if (required <= 0) return 0;
    return Math.min(100, (progress.current_minutes_watched / required) * 100);
}
