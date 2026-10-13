import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { RoomModeDetails, RoomModeIndicator } from './RoomModeIndicator';
import type { RoomModeView } from '../../stores/chatConnectionStore';

// The shape Rust ships for followers-only (10 minutes) plus slow mode (30 s)
// (services/room_modes.rs, test several_modes_list_most_limiting_first).
const followersAndSlow: RoomModeView = {
  modes: [
    {
      kind: 'followers_only',
      label: 'Followers-only chat (10 minutes)',
      compact: 'Followers-only (10m)',
      short: 'Followers 10m',
      detail: 'Only accounts that have followed for 10 minutes can chat.',
      applies_to_you: null,
    },
    {
      kind: 'slow',
      label: 'Slow mode (30 seconds)',
      compact: 'Slow mode (30s)',
      short: 'Slow 30s',
      detail: 'Chatters wait 30 seconds between messages.',
      applies_to_you: true,
    },
  ],
  chip: 'Followers 10m',
  exempt: false,
  prompt: 'Followers-only (10m)',
  prompt_short: 'Followers 10m',
  standing: null,
  slow_seconds: 30,
};

const asModerator: RoomModeView = {
  ...followersAndSlow,
  modes: followersAndSlow.modes.map((m) => ({ ...m, applies_to_you: false })),
  exempt: true,
  prompt: null,
  prompt_short: null,
  standing: 'You can chat as a moderator',
  slow_seconds: 0,
};

describe('RoomModeIndicator', () => {
  it('draws one chip naming the most limiting mode, a count for the rest and every mode for assistive tech', () => {
    const html = renderToStaticMarkup(<RoomModeIndicator view={followersAndSlow} />);
    expect(html).toContain('aria-label="Followers-only chat (10 minutes), Slow mode (30 seconds)"');
    expect(html).toContain('Followers 10m');
    expect(html).not.toContain('Slow 30s');
    expect(html).toContain('+1');
    expect(html).not.toContain('<svg');
    expect(html).toContain('text-textSecondary');
  });

  it('steps back a shade and says you can chat when every mode spares you', () => {
    const html = renderToStaticMarkup(<RoomModeIndicator view={asModerator} />);
    expect(html).toContain('text-textMuted');
    expect(html).toContain('You can chat as a moderator');
  });

  it('is a tappable flex item on the phone', () => {
    const html = renderToStaticMarkup(<RoomModeIndicator view={followersAndSlow} inline onActivate={() => {}} />);
    expect(html).toContain('role="button"');
    expect(html).not.toContain('absolute');
    expect(html).toContain('+1');
  });

  it('draws nothing for an open room', () => {
    expect(renderToStaticMarkup(<RoomModeIndicator view={{ ...followersAndSlow, modes: [], chip: null }} />)).toBe('');
  });
});

describe('RoomModeDetails', () => {
  it('lists each mode with its sentence and the standing line', () => {
    const html = renderToStaticMarkup(<RoomModeDetails view={asModerator} />);
    expect(html).toContain('Followers-only chat (10 minutes)');
    expect(html).toContain('Chatters wait 30 seconds between messages.');
    expect(html).toContain('You can chat as a moderator');
  });
});
