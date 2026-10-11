import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { membershipStatusCopy, shortDate, type MembershipStatus } from './membershipStatus';

const fmt = (iso: string) => iso.slice(0, 10);

const base: MembershipStatus = {
    kind: 'none',
    processor: null,
    giftPermanent: false,
    giftStartsAt: null,
    giftEndsAt: null,
    billingNote: null,
    billingResumesAt: null,
    renewsAt: null,
    paidEndsAt: null,
};
const s = (over: Partial<MembershipStatus>): MembershipStatus => ({ ...base, ...over });

describe('membershipStatusCopy', () => {
    it('says nothing without a membership', () => {
        expect(membershipStatusCopy(base, fmt)).toBeNull();
    });

    it('plain paid renews', () => {
        expect(membershipStatusCopy(s({ kind: 'paid', billingNote: 'renews', renewsAt: '2027-01-12T00:00:00Z' }), fmt))
            .toEqual({ label: 'Paid', tone: 'ok', headline: 'Paid membership · renews 2027-01-12', detail: null });
    });

    it('paid set to cancel', () => {
        const c = membershipStatusCopy(s({ kind: 'paid', billingNote: 'ends_at_period_end', paidEndsAt: '2027-01-12T00:00:00Z' }), fmt);
        expect(c?.headline).toBe('Paid membership · ends 2027-01-12');
        expect(c?.detail).toBe('Nothing more will be charged.');
        expect(c?.tone).toBe('warn');
    });

    it('paid past due', () => {
        expect(membershipStatusCopy(s({ kind: 'paid', billingNote: 'past_due' }), fmt)?.label).toBe('Past due');
    });

    it('gift with no billing set up', () => {
        const c = membershipStatusCopy(s({ kind: 'gifted', billingNote: 'none_set_up', giftEndsAt: '2027-03-03T00:00:00Z' }), fmt);
        expect(c).toEqual({
            label: 'Gifted',
            tone: 'gift',
            headline: 'Gifted membership · ends 2027-03-03',
            detail: 'No billing set up, it simply ends then.',
        });
    });

    it('permanent gift', () => {
        const c = membershipStatusCopy(s({ kind: 'gifted', billingNote: 'none_set_up', giftPermanent: true }), fmt);
        expect(c?.headline).toBe('Gifted membership · permanent');
        expect(c?.detail).toBe('No billing set up, nothing is ever charged.');
    });

    it('gift over a paused card subscription, not started yet', () => {
        const c = membershipStatusCopy(s({
            kind: 'gifted_over_paid',
            processor: 'stripe',
            billingNote: 'paused',
            giftStartsAt: '2026-11-01T00:00:00Z',
            giftEndsAt: '2027-02-01T00:00:00Z',
            billingResumesAt: '2027-02-01T00:00:00Z',
        }), fmt);
        expect(c?.headline).toBe('Gifted membership · starts 2026-11-01, ends 2027-02-01');
        expect(c?.detail).toBe('Your billing is paused and resumes on its own on 2027-02-01.');
    });

    it('gift over PayPal says billing continues', () => {
        const c = membershipStatusCopy(s({ kind: 'gifted_over_paid', processor: 'paypal', billingNote: 'continues_paypal', giftEndsAt: '2027-03-03T00:00:00Z' }), fmt);
        expect(c?.detail).toBe('Your PayPal billing continues as usual.');
    });

    it('gift over a card subscription that was not paused (or a refund took the pause back)', () => {
        const c = membershipStatusCopy(s({ kind: 'gifted_over_paid', processor: 'stripe', billingNote: 'continues', giftEndsAt: '2027-03-03T00:00:00Z' }), fmt);
        expect(c?.detail).toBe('Your card billing continues as usual.');
    });

    it('gift over a subscription set to cancel', () => {
        const c = membershipStatusCopy(s({ kind: 'gifted_over_paid', billingNote: 'ends_at_period_end', giftEndsAt: '2027-03-03T00:00:00Z' }), fmt);
        expect(c?.detail).toBe('Your paid membership is set to cancel, so nothing will be charged.');
    });

    it('a note from a newer server still shows the headline', () => {
        const c = membershipStatusCopy(s({ kind: 'gifted', billingNote: 'unknown', giftEndsAt: '2027-03-03T00:00:00Z' }), fmt);
        expect(c?.headline).toBe('Gifted membership · ends 2027-03-03');
        expect(c?.detail).toBeNull();
    });

    it('boost', () => {
        expect(membershipStatusCopy(s({ kind: 'boost' }), fmt)?.headline).toBe('Membership from your Discord boost');
    });
});

describe('shortDate', () => {
    it('drops the year inside the current year and keeps it otherwise', () => {
        const now = new Date('2026-10-10T12:00:00');
        expect(shortDate('2026-11-03T12:00:00', now, 'en-US')).toBe('Nov 3');
        expect(shortDate('2027-03-03T12:00:00', now, 'en-US')).toBe('Mar 3, 2027');
    });
});
