// What each platform calls the two relationships a viewer can have with a channel,
// so the UI uses the words that platform's own users use.
//
// The two concepts are the same everywhere; only the vocabulary moves, and on
// YouTube it moves in a way that INVERTS Twitch's:
//
//   free relationship   Twitch/Kick "Follow"     YouTube "Subscribe"
//   paid relationship   Twitch/Kick "Subscribe"  YouTube "Join" (a membership)
//
// So a button labelled "Subscribe" on a YouTube stream reads to a YouTube viewer
// as the FREE action, while it opens the paid one. Hardcoding Twitch's words is
// not just off-brand there, it is actively misleading.

import type { ProviderId } from '../types/providers';

export interface PlatformTerms {
    /** The free relationship, as a verb. */
    follow: string;
    /** The free relationship, once you have it. */
    following: string;
    /** Undoing the free relationship. */
    unfollow: string;
    /** The paid relationship, as a verb. */
    paid: string;
    /** Renewing a lapsed paid relationship. */
    paidAgain: string;
    /** Giving the paid relationship to someone else. */
    paidGift: string;
    /** Someone who holds the paid relationship. */
    paidHolder: string;
}

const TWITCH_LIKE: PlatformTerms = {
    follow: 'Follow',
    following: 'Following',
    unfollow: 'Unfollow',
    paid: 'Subscribe',
    paidAgain: 'Resubscribe',
    paidGift: 'Gift Subs',
    paidHolder: 'subscriber',
};

const YOUTUBE: PlatformTerms = {
    follow: 'Subscribe',
    following: 'Subscribed',
    unfollow: 'Unsubscribe',
    // YouTube's paid tier is a channel MEMBERSHIP, and its button says "Join".
    paid: 'Join',
    paidAgain: 'Rejoin',
    paidGift: 'Gift membership',
    paidHolder: 'member',
};

export function platformTerms(provider: ProviderId | string | undefined): PlatformTerms {
    return provider === 'youtube' ? YOUTUBE : TWITCH_LIKE;
}

export default platformTerms;
