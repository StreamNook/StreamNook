import { describe, expect, it } from 'vitest';
import type { PageLink, PageLinkAnswer } from '../../types/profileView';
import { desktopPlateSlot, profileLinkAsk, profileLinkKey } from './profileLink';

const link: PageLink = {
  href: 'https://streamnook.app/u/tyler',
  url: 'https://streamnook.app/tyler',
  prefix: 'streamnook.app/',
  name: 'tyler',
  accent: '#22d3ee',
};
const member: PageLinkAnswer = { state: 'member', link };
const notMember: PageLinkAnswer = { state: 'not_member' };
const notReady: PageLinkAnswer = { state: 'not_ready' };

describe('profileLinkAsk', () => {
  it('asks with the member id and the member own login', () => {
    expect(profileLinkAsk('676', 'tyler')).toEqual({ memberId: '676', login: 'tyler' });
  });

  it('trims, and asks nothing while either part is missing', () => {
    expect(profileLinkAsk(' 676 ', ' tyler ')).toEqual({ memberId: '676', login: 'tyler' });
    expect(profileLinkAsk('676', undefined)).toBeNull();
    expect(profileLinkAsk('676', '')).toBeNull();
    expect(profileLinkAsk(null, 'tyler')).toBeNull();
  });

  it('keys an answer by the question it was for', () => {
    expect(profileLinkKey('676', 'tyler')).toBe('676|tyler');
    expect(profileLinkKey('676', '')).toBeNull();
  });
});

describe('desktopPlateSlot', () => {
  const base = { answer: null, asked: true, loadFinished: false, expectMember: true };

  it('shows a link whatever else is true', () => {
    expect(desktopPlateSlot({ ...base, answer: member })).toBe('shown');
    expect(desktopPlateSlot({ ...base, answer: member, expectMember: false, loadFinished: true })).toBe('shown');
  });

  it('shows nothing for a "not a member" answer', () => {
    expect(desktopPlateSlot({ ...base, answer: notMember })).toBe('hidden');
  });

  it('holds no space on a surface that does not expect a member', () => {
    expect(desktopPlateSlot({ ...base, expectMember: false })).toBe('hidden');
  });

  it('holds the slot while a member answer is on its way', () => {
    expect(desktopPlateSlot(base)).toBe('reserved');
    expect(desktopPlateSlot({ ...base, loadFinished: true })).toBe('reserved');
  });

  it('holds the slot until the login arrives, then lets go once loading is over', () => {
    expect(desktopPlateSlot({ ...base, asked: false })).toBe('reserved');
    expect(desktopPlateSlot({ ...base, asked: false, loadFinished: true })).toBe('hidden');
  });

  it('lets a "not ready" answer go only after the surface finished loading', () => {
    expect(desktopPlateSlot({ ...base, answer: notReady })).toBe('reserved');
    expect(desktopPlateSlot({ ...base, answer: notReady, loadFinished: true })).toBe('hidden');
  });

  it('lets a member answer with no link go once loaded (nothing can come)', () => {
    expect(desktopPlateSlot({ ...base, answer: { state: 'member' }, loadFinished: true })).toBe('hidden');
  });
});
