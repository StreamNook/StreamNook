import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const { invoke } = await import('@tauri-apps/api/core');
const { lookupError, resolveKickSlug, resolveYouTubeIdentifier, youTubeChannelTitle } = await import(
  './channelLookup'
);

/**
 * Only an input the text can't settle costs a round trip: a legacy YouTube
 * /c/ or /user/ path, or a Kick name with a second spelling. Everything else a
 * parser returns is already what gets stored, and must not wait on the network.
 */
describe('resolveYouTubeIdentifier', () => {
  it('hands anything but a legacy path straight back, without asking Rust', async () => {
    vi.mocked(invoke).mockClear();
    for (const id of ['@LofiGirl', 'UCSJ4gkVC6NrvII8umztf0Ow', 'jfKfPfyJRdk']) {
      await expect(resolveYouTubeIdentifier(id)).resolves.toBe(id);
    }
    expect(invoke).not.toHaveBeenCalled();
  });

  it('asks Rust which channel a legacy path is, and returns its UC id', async () => {
    vi.mocked(invoke).mockResolvedValueOnce('UCX6OQ3DkcsbYNE6H8uQQuVA');
    await expect(resolveYouTubeIdentifier('user/MrBeast6000')).resolves.toBe('UCX6OQ3DkcsbYNE6H8uQQuVA');
    expect(invoke).toHaveBeenCalledWith('resolve_youtube_legacy_channel', { path: 'user/MrBeast6000' });
  });
});

describe('resolveKickSlug', () => {
  it('hands a name with one spelling straight back, without asking Rust', async () => {
    vi.mocked(invoke).mockClear();
    await expect(resolveKickSlug('xqc')).resolves.toBe('xqc');
    expect(invoke).not.toHaveBeenCalled();
  });

  it('asks Rust how Kick spells a name with an underscore or hyphen', async () => {
    vi.mocked(invoke).mockResolvedValueOnce('some-name');
    await expect(resolveKickSlug('some_name')).resolves.toBe('some-name');
    expect(invoke).toHaveBeenCalledWith('resolve_kick_slug', { name: 'some_name' });
  });
});

describe('youTubeChannelTitle', () => {
  it('asks once per id and remembers the answer', async () => {
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockResolvedValueOnce({ title: 'Lofi Girl' });
    await expect(youTubeChannelTitle('UCSJ4gkVC6NrvII8umztf0Ow')).resolves.toBe('Lofi Girl');
    await expect(youTubeChannelTitle('UCSJ4gkVC6NrvII8umztf0Ow')).resolves.toBe('Lofi Girl');
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('youtube_user_profile', { channelId: 'UCSJ4gkVC6NrvII8umztf0Ow' });
  });

  it('forgets a failed request, so the next call can try again', async () => {
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockRejectedValueOnce('YouTube returned 503');
    await expect(youTubeChannelTitle('UCX6OQ3DkcsbYNE6H8uQQuVA')).resolves.toBeNull();
    vi.mocked(invoke).mockResolvedValueOnce({ title: 'MrBeast' });
    await expect(youTubeChannelTitle('UCX6OQ3DkcsbYNE6H8uQQuVA')).resolves.toBe('MrBeast');
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it('reads a blank title as no name', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ title: '   ' });
    await expect(youTubeChannelTitle('UCaaaaaaaaaaaaaaaaaaaaaa')).resolves.toBeNull();
  });
});

describe('lookupError', () => {
  it("shows Rust's own wording, and a readable line when there is none", () => {
    expect(lookupError('YouTube has no channel at that link.', 'YouTube')).toBe(
      'YouTube has no channel at that link.',
    );
    expect(lookupError(new Error('socket hang up'), 'Kick')).toBe(
      "Couldn't look that Kick channel up. Please try again.",
    );
  });
});
