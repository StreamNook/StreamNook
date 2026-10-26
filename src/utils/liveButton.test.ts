import { describe, expect, it } from 'vitest';
import { initialLiveButtonState, type LiveButtonState } from './liveButton';

const LASTS: LiveButtonState[] = ['live', 'behind', 'catching-up'];

describe('initialLiveButtonState', () => {
  it.each(LASTS)('is behind when rewound, whatever it last was (%s)', (last) => {
    expect(initialLiveButtonState({ rewound: true, paused: false, last })).toBe('behind');
  });

  it.each(LASTS)('is behind when paused, whatever it last was (%s)', (last) => {
    expect(initialLiveButtonState({ rewound: false, paused: true, last })).toBe('behind');
  });

  it.each(LASTS)('is behind when rewound and paused (%s)', (last) => {
    expect(initialLiveButtonState({ rewound: true, paused: true, last })).toBe('behind');
  });

  it.each(LASTS)('keeps the last state when neither (%s)', (last) => {
    expect(initialLiveButtonState({ rewound: false, paused: false, last })).toBe(last);
  });
});
