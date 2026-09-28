import { describe, expect, it } from 'vitest';
import type { DropProgress, TwitchProgress } from '../types';
import { claimLabel, dropPercent, dropRequirementText, resetsInLabel, twitchProgressLine } from './dropRequirement';

const NOW = Date.parse('2026-09-25T12:00:00Z');

function progress(t: Partial<TwitchProgress>, extra: Partial<DropProgress> = {}): DropProgress {
    return {
        campaign_id: 'c',
        drop_id: 'd',
        current_minutes_watched: 25,
        required_minutes_watched: 60,
        is_claimed: false,
        last_updated: '2026-09-25T12:00:00Z',
        twitch_progress: {
            days_done: 1,
            minutes_today: 5,
            window_expires_at: '2026-09-26T11:00:00Z',
            accruing: false,
            subs_done: 0,
            ready_to_claim: false,
            earned: null,
            ...t,
        },
        ...extra,
    };
}

const daily = { required_minutes_watched: 60, required_days: 3, required_subs: 0 };
const sub2 = { required_minutes_watched: 0, required_days: 0, required_subs: 2 };

describe('dropRequirementText', () => {
    it('states multi-day and subscription requirements', () => {
        expect(dropRequirementText(daily)).toBe('20 min a day · 3 days');
        expect(dropRequirementText(sub2)).toBe('2 subs or gift subs');
        expect(dropRequirementText({ ...sub2, required_subs: 1 })).toBe('Subscribe or gift a sub');
    });
    it('leaves plain watch time to the minutes', () => {
        expect(dropRequirementText({ required_minutes_watched: 240 })).toBeNull();
    });
});

describe('twitchProgressLine', () => {
    it('reads a day in progress', () => {
        expect(twitchProgressLine(progress({}), daily, NOW)).toBe('Day 2 of 3 · 5/20 min today · resets in 23h');
    });
    it('names what came out of a random draw', () => {
        const got = progress({ earned: { id: 'b', name: 'Bulbasaur', image_url: '' } }, { is_claimed: true });
        expect(twitchProgressLine(got, daily, NOW)).toBe('You got Bulbasaur');
    });
    it('names a reward waiting to be claimed', () => {
        const held = progress({ ready_to_claim: true, earned: { id: 's', name: 'Sierra Helmet', image_url: '' } });
        expect(twitchProgressLine(held, { required_minutes_watched: 240 }, NOW)).toBe('Sierra Helmet ready to claim');
        expect(claimLabel({ required_minutes_watched: 240 }, held)).toBe('Claim');
    });
    it('opens a ball rather than claiming it', () => {
        const unlocked = progress({ ready_to_claim: true });
        expect(twitchProgressLine(unlocked, { ...daily, random_of: 3 }, NOW)).toBe('Ready to open');
        expect(claimLabel({ ...daily, random_of: 3 }, unlocked)).toBe('Open');
    });
    it('counts subs without overshooting', () => {
        expect(twitchProgressLine(progress({ subs_done: 1 }), sub2, NOW)).toBe('1 of 2 subs');
        expect(twitchProgressLine(progress({ subs_done: 4 }), sub2, NOW)).toBe('2 of 2 subs');
    });
    it('never claims a day past the last', () => {
        expect(twitchProgressLine(progress({ days_done: 3, window_expires_at: null }), daily, NOW)).toBe('Day 3 of 3 · 5/20 min today');
    });
    it('counts a finished day the way Twitch does', () => {
        // Twitch: "20/20m (Day 2)" at days_done 2.
        const met = progress({ days_done: 2, minutes_today: 20, window_expires_at: '2026-09-26T09:00:00Z' });
        expect(twitchProgressLine(met, daily, NOW)).toBe('Day 2 of 3 · done for today · next day in 21h');
    });
    it('is silent without Twitch detail', () => {
        expect(twitchProgressLine({ ...progress({}), twitch_progress: null }, daily, NOW)).toBeNull();
    });
});

describe('dropPercent and resetsInLabel', () => {
    it('fills a held or subscribed reward', () => {
        expect(dropPercent(progress({ ready_to_claim: true }), sub2)).toBe(100);
        expect(dropPercent(progress({ subs_done: 1 }), sub2)).toBe(50);
        expect(Math.round(dropPercent(progress({}), daily))).toBe(42);
    });
    it('labels the window reset', () => {
        expect(resetsInLabel('2026-09-25T12:40:00Z', NOW)).toBe('in 40m');
        expect(resetsInLabel('2026-09-25T11:00:00Z', NOW)).toBe('soon');
    });
});
