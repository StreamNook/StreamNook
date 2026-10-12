import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { RoomModeDetails, RoomModeIndicator } from './RoomModeIndicator';
import { roomModesFieldStyle, roomModesPad } from './roomModeLayout';
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
  prompt: 'Followers-only (10m)',
  prompt_short: 'Followers 10m',
  standing: null,
  slow_seconds: 30,
};

const asModerator: RoomModeView = {
  ...followersAndSlow,
  modes: followersAndSlow.modes.map((m) => ({ ...m, applies_to_you: false })),
  prompt: null,
  prompt_short: null,
  standing: 'You can chat as a moderator',
  slow_seconds: 0,
};

describe('roomModesPad', () => {
  it('reserves nothing extra with no modes', () => {
    expect(roomModesPad(0)).toEqual({ wide: 12, narrow: 12 });
    expect(roomModesFieldStyle(null)).toBeUndefined();
    expect(roomModesFieldStyle({ ...followersAndSlow, modes: [] })).toBeUndefined();
  });

  it('reserves every icon when wide and one icon plus a count when narrow', () => {
    // 8 edge + 13 icon + 6 clear
    expect(roomModesPad(1)).toEqual({ wide: 27, narrow: 27 });
    // 8 + 2*13 + 4 + 6 wide; 8 + 13 + 4 + 13 + 6 narrow
    expect(roomModesPad(2)).toEqual({ wide: 44, narrow: 44 });
    expect(roomModesPad(5)).toEqual({ wide: 95, narrow: 44 });
  });

  it('hands the widths to the field as CSS variables', () => {
    expect(roomModesFieldStyle(followersAndSlow)).toEqual({
      '--sn-room-pad-wide': '44px',
      '--sn-room-pad-narrow': '44px',
    });
  });
});

describe('RoomModeIndicator', () => {
  it('draws one icon per mode, names them for assistive tech and counts the rest for a narrow box', () => {
    const html = renderToStaticMarkup(<RoomModeIndicator view={followersAndSlow} />);
    expect(html).toContain('aria-label="Followers-only chat (10 minutes), Slow mode (30 seconds)"');
    expect(html.match(/<svg/g)).toHaveLength(2);
    expect(html).toContain('data-room-mode-rest=""');
    expect(html).toContain('+1');
    expect(html).toContain('text-textSecondary');
  });

  it('steps back a shade and says you can chat when every mode spares you', () => {
    const html = renderToStaticMarkup(<RoomModeIndicator view={asModerator} />);
    expect(html).toContain('text-textMuted');
    expect(html).toContain('You can chat as a moderator');
  });

  it('keeps the first icon and the count inline on the phone', () => {
    const html = renderToStaticMarkup(<RoomModeIndicator view={followersAndSlow} inline onActivate={() => {}} />);
    expect(html.match(/<svg/g)).toHaveLength(1);
    expect(html).toContain('role="button"');
    expect(html).toContain('+1');
    expect(html).not.toContain('data-room-mode-count');
  });

  it('draws nothing for an open room', () => {
    expect(renderToStaticMarkup(<RoomModeIndicator view={{ ...followersAndSlow, modes: [] }} />)).toBe('');
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
