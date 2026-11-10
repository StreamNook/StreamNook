import { describe, expect, it } from 'vitest';
import { channelToolSections, type ChannelToolContext } from './channelTools';

const viewer: ChannelToolContext = {
  isTwitch: true,
  isModerator: false,
  isVodReplay: false,
  manages: 'no',
  hasDropsCampaign: false,
};

describe('channelToolSections', () => {
  it('gives a plain viewer no Tools button', () => {
    expect(channelToolSections(viewer)).toEqual({ moderate: false, polls: false, drops: false, any: false });
  });

  it('shows drops alone to a viewer collecting them', () => {
    const s = channelToolSections({ ...viewer, hasDropsCampaign: true });
    expect(s).toEqual({ moderate: false, polls: false, drops: true, any: true });
  });

  it('gives a Twitch moderator moderation and the stream manager rows', () => {
    const s = channelToolSections({ ...viewer, isModerator: true, manages: 'yes' });
    expect(s.moderate).toBe(true);
    expect(s.polls).toBe(true);
    expect(s.any).toBe(true);
    expect(channelToolSections({ ...viewer, isModerator: true, manages: 'yes', isVodReplay: true }).polls).toBe(false);
  });

  it('gives an editor (no badge) the stream manager rows once Rust says so', () => {
    const editor = channelToolSections({ ...viewer, manages: 'yes' });
    expect(editor).toEqual({ moderate: false, polls: true, drops: false, any: true });
    // Not while it is still unknown.
    expect(channelToolSections({ ...viewer, manages: 'unknown' }).any).toBe(false);
  });

  it('gives the broadcaster polls, but not while replaying a VOD', () => {
    expect(channelToolSections({ ...viewer, manages: 'yes' }).polls).toBe(true);
    expect(channelToolSections({ ...viewer, manages: 'yes', isVodReplay: true }).polls).toBe(false);
  });

  it('keeps the Helix-only sections off Kick and YouTube', () => {
    const s = channelToolSections({ ...viewer, isTwitch: false, isModerator: true, manages: 'yes' });
    expect(s).toEqual({ moderate: false, polls: false, drops: false, any: false });
  });
});
