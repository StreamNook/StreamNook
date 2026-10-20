import { describe, expect, it, vi } from 'vitest';
import { authFailureOutcome, isNetworkAuthError, PROFILE_TRIES, readProfile } from './authErrors';

// Rust's words for a failed refresh (`refresh_failure_message` in
// src-tauri/src/services/twitch_service.rs).
const NETWORK_REFRESH =
  'Token refresh failed with a network error; the sign-in is kept and tried again (refreshing the Twitch token: error sending request for url (https://id.twitch.tv/oauth2/token))';
const REFUSED_REFRESH =
  'Token expired and refresh failed (Twitch refused the token refresh (400): {"status":400,"message":"Invalid refresh token"}). Please log in again.';

describe('authFailureOutcome', () => {
  it('keeps the session when the refresh never reached Twitch', () => {
    expect(isNetworkAuthError(NETWORK_REFRESH)).toBe(true);
    expect(authFailureOutcome({ hasCredentials: true, message: NETWORK_REFRESH, missingScopes: false })).toBe('keep-session');
  });

  it('signs out when Twitch turned the refresh token down', () => {
    expect(isNetworkAuthError(REFUSED_REFRESH)).toBe(false);
    expect(authFailureOutcome({ hasCredentials: true, message: REFUSED_REFRESH, missingScopes: false })).toBe('signed-out');
  });

  it('signs out without a stored token, or when Rust ended the sign-in itself', () => {
    expect(authFailureOutcome({ hasCredentials: false, message: NETWORK_REFRESH, missingScopes: false })).toBe('signed-out');
    expect(authFailureOutcome({ hasCredentials: true, message: NETWORK_REFRESH, missingScopes: true })).toBe('signed-out');
  });

  it('reads a Helix outage as the network', () => {
    expect(authFailureOutcome({ hasCredentials: true, message: 'Helix answered 503 Service Unavailable', missingScopes: false })).toBe('keep-session');
  });
});

describe('readProfile', () => {
  it('tries again a second later, as the iPhone does', async () => {
    const wait = vi.fn(async () => {});
    const read = vi
      .fn<() => Promise<string>>()
      .mockRejectedValueOnce(new Error(NETWORK_REFRESH))
      .mockResolvedValueOnce('me');
    await expect(readProfile(read, wait)).resolves.toBe('me');
    expect(read).toHaveBeenCalledTimes(2);
    expect(wait).toHaveBeenCalledWith(1000);
  });

  it('gives up after three tries with the last error', async () => {
    const wait = vi.fn(async () => {});
    const read = vi.fn<() => Promise<string>>().mockRejectedValue(new Error(REFUSED_REFRESH));
    await expect(readProfile(read, wait)).rejects.toThrow('Please log in again');
    expect(read).toHaveBeenCalledTimes(PROFILE_TRIES);
    expect(wait).toHaveBeenCalledTimes(PROFILE_TRIES - 1);
  });
});
