import { describe, expect, it } from 'vitest';
import { categoryOf, chatEventTemplateContext, eventKeyOf, isHiddenEvent } from './chatEvents';
import { renderEventTemplate } from '../components/overlay/overlayConfig';

describe('chat events', () => {
  it('maps notice ids to categories', () => {
    expect(categoryOf('resub')).toBe('subscription');
    expect(categoryOf('submysterygift')).toBe('gift');
    expect(categoryOf('raid')).toBe('raid');
    expect(categoryOf('viewermilestone')).toBe('milestone');
    expect(categoryOf(undefined)).toBeNull();
    expect(categoryOf('nothing')).toBeNull();
  });

  it('keys an event by platform and category, and promotes a Twitch cheer', () => {
    expect(eventKeyOf({ provider: 'twitch', metadata: { msg_type: 'subgift' } })).toBe('twitch:gift');
    expect(eventKeyOf({ provider: 'kick', tags: { 'msg-id': 'kick_follow' } })).toBe('kick:follow');
    expect(eventKeyOf({ provider: 'twitch', metadata: { bits_amount: 100 } })).toBe('twitch:cheer');
    expect(eventKeyOf({ provider: 'twitch', metadata: {} })).toBeNull();
  });

  it('hides only what the list names', () => {
    const m = { provider: 'twitch', metadata: { msg_type: 'raid' } };
    expect(isHiddenEvent(m, ['twitch:raid'])).toBe(true);
    expect(isHiddenEvent(m, ['kick:raid'])).toBe(false);
    expect(isHiddenEvent(m, [])).toBe(false);
    expect(isHiddenEvent({ provider: 'twitch' }, ['twitch:raid'])).toBe(false);
  });

  it('builds a template context that fills the overlay template', () => {
    const tags = new Map([
      ['msg-param-cumulative-months', '14'],
      ['msg-param-sub-plan', '2000'],
    ]);
    const ctx = chatEventTemplateContext(tags, 'subscription', { username: 'brandon', displayName: 'Brandon' });
    expect(ctx.months).toBe(14);
    expect(ctx.years).toBe(1);
    expect(ctx.tier).toBe('Tier 2');
    expect(renderEventTemplate('{username} is on month {months} ({tier})', ctx)).toBe('Brandon is on month 14 (Tier 2)');
    // A token the event does not carry fails the whole template, so the
    // platform wording is used instead of a sentence with a hole in it.
    expect(renderEventTemplate('{username} gifted {count}', ctx)).toBeNull();
  });
});
