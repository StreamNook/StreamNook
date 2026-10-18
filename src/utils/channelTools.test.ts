import { describe, expect, it } from 'vitest';
import { channelToolSections, type ChannelToolContext } from './channelTools';

const viewer: ChannelToolContext = {
  isTwitch: true,
  isModerator: false,
  isBroadcaster: false,
  isVodReplay: false,
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
    const s = channelToolSections({ ...viewer, isModerator: true });
    expect(s.moderate).toBe(true);
    expect(s.polls).toBe(true);
    expect(s.any).toBe(true);
    expect(channelToolSections({ ...viewer, isModerator: true, isVodReplay: true }).polls).toBe(false);
  });

  it('gives the broadcaster polls, but not while replaying a VOD', () => {
    expect(channelToolSections({ ...viewer, isModerator: true, isBroadcaster: true }).polls).toBe(true);
    expect(channelToolSections({ ...viewer, isBroadcaster: true, isVodReplay: true }).polls).toBe(false);
  });

  it('keeps the Helix-only sections off Kick and YouTube', () => {
    const s = channelToolSections({ ...viewer, isTwitch: false, isModerator: true, isBroadcaster: true });
    expect(s).toEqual({ moderate: false, polls: false, drops: false, any: false });
  });
});
