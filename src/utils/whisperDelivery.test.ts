import { describe, expect, it } from 'vitest';
import { DELIVERY_LABEL, cutLine, deliveryOf, replyRefused } from './whisperDelivery';

describe('whisper delivery words', () => {
  it('reads a state this build does not know as Sent', () => {
    expect(deliveryOf({ delivery: 'queued_somewhere_new' })).toBe('sent');
  });

  it('has no state for a message that is not one of our sends', () => {
    expect(deliveryOf({ delivery: null })).toBeNull();
  });

  it('keeps every known state', () => {
    for (const state of ['sending', 'sent', 'unknown', 'delivered', 'not_delivered'] as const) {
      expect(deliveryOf({ delivery: state })).toBe(state);
    }
  });

  it('says Checking while Rust looks for Twitch\'s copy', () => {
    expect(DELIVERY_LABEL.unknown).toBe('Checking...');
  });

  it('words the cut line and the refused reply', () => {
    expect(cutLine(500)).toBe('Twitch cut this whisper to 500 characters.');
    expect(replyRefused('Someone')).toBe('This whisper came to Someone. Switch to that account to reply.');
  });
});
