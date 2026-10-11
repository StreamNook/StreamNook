import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

// The member's own membership status line. streamnook.app computes it and Rust
// (services/membership_status.rs) fetches and caches it; this file only turns
// it into words. It lives in TS because the dates are formatted in the
// viewer's locale. The wording matches the streamnook.app account page.

export type MembershipKind = 'paid' | 'gifted' | 'gifted_over_paid' | 'boost' | 'none';

export type BillingNote =
    | 'paused'
    | 'continues'
    | 'continues_paypal'
    | 'none_set_up'
    | 'ends_at_period_end'
    | 'renews'
    | 'past_due'
    | 'unknown';

export interface MembershipStatus {
    kind: MembershipKind;
    processor: string | null;
    giftPermanent: boolean;
    giftStartsAt: string | null;
    giftEndsAt: string | null;
    billingNote: BillingNote | null;
    billingResumesAt: string | null;
    renewsAt: string | null;
    paidEndsAt: string | null;
}

export interface MembershipStatusCopy {
    /** One word beside the status dot. */
    label: string;
    tone: 'gift' | 'ok' | 'warn';
    headline: string;
    detail: string | null;
}

export const getMembershipStatus = (force = false): Promise<MembershipStatus | null> =>
    invoke<MembershipStatus | null>('get_membership_status', { force }).catch(() => null);

export const onMembershipStatusChanged = (cb: (status: MembershipStatus) => void): Promise<UnlistenFn> =>
    listen<MembershipStatus>('membership-status-changed', (e) => cb(e.payload));

/** "Mar 3", or "Mar 3, 2027" when the date falls in another year. */
export const shortDate = (iso: string, now: Date = new Date(), locale?: string): string => {
    const d = new Date(iso);
    const sameYear = d.getFullYear() === now.getFullYear();
    return d.toLocaleDateString(
        locale,
        sameYear ? { month: 'short', day: 'numeric' } : { month: 'short', day: 'numeric', year: 'numeric' },
    );
};

export const membershipStatusCopy = (
    s: MembershipStatus,
    fmt: (iso: string) => string = (iso) => shortDate(iso),
): MembershipStatusCopy | null => {
    switch (s.kind) {
        case 'gifted':
        case 'gifted_over_paid': {
            const headline = s.giftPermanent || !s.giftEndsAt
                ? 'Gifted membership · permanent'
                : s.giftStartsAt
                    ? `Gifted membership · starts ${fmt(s.giftStartsAt)}, ends ${fmt(s.giftEndsAt)}`
                    : `Gifted membership · ends ${fmt(s.giftEndsAt)}`;
            let detail: string | null;
            switch (s.billingNote) {
                case 'paused':
                    detail = s.billingResumesAt
                        ? `Your billing is paused and resumes on its own on ${fmt(s.billingResumesAt)}.`
                        : 'Your billing is paused for the gift.';
                    break;
                case 'continues_paypal':
                    detail = 'Your PayPal billing continues as usual.';
                    break;
                case 'continues':
                    detail = 'Your card billing continues as usual.';
                    break;
                case 'ends_at_period_end':
                    detail = 'Your paid membership is set to cancel, so nothing will be charged.';
                    break;
                case 'none_set_up':
                    detail = s.giftPermanent
                        ? 'No billing set up, nothing is ever charged.'
                        : 'No billing set up, it simply ends then.';
                    break;
                default:
                    detail = null;
            }
            return { label: 'Gifted', tone: 'gift', headline, detail };
        }
        case 'paid':
            if (s.billingNote === 'past_due') {
                return {
                    label: 'Past due',
                    tone: 'warn',
                    headline: 'Paid membership · last payment did not go through',
                    detail: 'Update your payment method to keep it.',
                };
            }
            if (s.billingNote === 'ends_at_period_end') {
                return {
                    label: 'Ending',
                    tone: 'warn',
                    headline: s.paidEndsAt ? `Paid membership · ends ${fmt(s.paidEndsAt)}` : 'Paid membership · ending',
                    detail: 'Nothing more will be charged.',
                };
            }
            return {
                label: 'Paid',
                tone: 'ok',
                headline: s.renewsAt ? `Paid membership · renews ${fmt(s.renewsAt)}` : 'Paid membership',
                detail: null,
            };
        case 'boost':
            return {
                label: 'Boost',
                tone: 'gift',
                headline: 'Membership from your Discord boost',
                detail: 'It lasts while you boost, nothing is charged.',
            };
        default:
            return null;
    }
};
