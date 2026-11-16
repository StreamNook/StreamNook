import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { gifSendMessage } from './gifService';

describe('gifSendMessage', () => {
  it('says nothing for a GIF that went out', () => {
    expect(gifSendMessage({ sent: true, error: null, seconds_until_can_send: 0 })).toBe('');
  });

  it("names Twitch's security check when it refused the send", () => {
    expect(gifSendMessage({ sent: false, error: 'IntegrityCheckFailed', seconds_until_can_send: 0 })).toMatch(/security check/);
  });

  it('keeps the cooldown and the generic refusal', () => {
    expect(gifSendMessage({ sent: false, error: 'RATE_LIMITED', seconds_until_can_send: 9 })).toMatch(/9s/);
    expect(gifSendMessage({ sent: false, error: 'GRAPHQL_ERROR', seconds_until_can_send: 0 })).toBe('That GIF could not be sent.');
  });
});
