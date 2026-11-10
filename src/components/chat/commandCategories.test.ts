import { describe, expect, it } from 'vitest';
import { COMMAND_DEFINITIONS } from '../../utils/chatCommands';
import { CATEGORY_ORDER, RAIL, lockFor } from './CommandMenu';

// Every category in the command table has a place in the command menu, so no
// command is ever left out of the list or its rail.

describe('command categories', () => {
  const used = [...new Set(COMMAND_DEFINITIONS.map((c) => c.category))];

  it('every category in the table is listed and has a rail entry', () => {
    for (const c of used) {
      expect(CATEGORY_ORDER, c).toContain(c);
      expect(RAIL.some((r) => r.key === c), c).toBe(true);
    }
  });

  it("the stream manager's commands sit under Channel and are never locked", () => {
    const channel = COMMAND_DEFINITIONS.filter((c) => c.category === 'Channel').map((c) => c.name);
    expect(channel).toEqual(
      expect.arrayContaining(['poll', 'prediction', 'settitle', 'setgame', 'endpoll', 'cancelpoll', 'lockprediction', 'cancelprediction', 'completeprediction']),
    );
    const poll = COMMAND_DEFINITIONS.find((c) => c.name === 'poll')!;
    expect(lockFor(poll, false, false)).toBeNull();
    const raid = COMMAND_DEFINITIONS.find((c) => c.name === 'raid')!;
    expect(lockFor(raid, true, false)?.line).toBe('Needs the broadcaster in this channel');
  });
});
