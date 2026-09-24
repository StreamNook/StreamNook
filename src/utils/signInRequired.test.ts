import { describe, expect, it } from 'vitest';
import { signInRequiredFrom } from './signInRequired';

describe('signInRequiredFrom', () => {
  it('reads the sign-in shape start_stream rejects with', () => {
    const e = { sign_in: { provider: 'tiktok', channel: 'someone', message: "@someone's LIVE is 18+." } };
    expect(signInRequiredFrom(e)).toEqual({
      provider: 'tiktok',
      channel: 'someone',
      message: "@someone's LIVE is 18+.",
    });
  });

  it('leaves every ordinary failure alone', () => {
    expect(signInRequiredFrom("@someone isn't live right now")).toBeNull();
    expect(signInRequiredFrom(new Error('boom'))).toBeNull();
    expect(signInRequiredFrom(null)).toBeNull();
    expect(signInRequiredFrom(undefined)).toBeNull();
  });

  it('refuses a payload it cannot act on', () => {
    // Twitch is the app's own account, not a platform sign-in.
    expect(signInRequiredFrom({ sign_in: { provider: 'twitch', channel: 'a', message: 'x' } })).toBeNull();
    expect(signInRequiredFrom({ sign_in: { provider: 'tiktok', channel: 'a', message: '' } })).toBeNull();
    expect(signInRequiredFrom({ sign_in: { provider: 'tiktok', message: 'x' } })).toBeNull();
    expect(signInRequiredFrom({ sign_in: 'tiktok' })).toBeNull();
  });
});
