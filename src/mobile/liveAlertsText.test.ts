import { describe, expect, it } from 'vitest';
import {
  copyErrorText,
  copyResultText,
  sheetIntro,
  undoResultText,
  whichChannelsDetail,
} from './liveAlertsText';

describe('Which channels wording', () => {
  it('counts only muted channels you follow once Rust answers', () => {
    expect(whichChannelsDetail(3, 9)).toBe('Everything you follow except 3');
    expect(whichChannelsDetail(0, 9)).toBe('Everything you follow');
  });

  it('falls back to the stored list while loading and when signed out', () => {
    expect(whichChannelsDetail(undefined, 4)).toBe('Everything you follow except 4');
    expect(whichChannelsDetail(null, 1)).toBe('1 channel muted');
    expect(whichChannelsDetail(null, 0)).toBe('Everything you follow');
  });

  it('opens the sheet with the same numbers', () => {
    expect(sheetIntro(2, 5)).toBe('Alerting for every channel you follow except 2.');
    expect(sheetIntro(0, 5)).toBe(
      'Alerting for every channel you follow. Turn off any you would rather not hear about.',
    );
    expect(sheetIntro(null, 2)).toBe('2 channels are muted.');
  });
});

describe('Copy from Twitch wording', () => {
  it('says what was muted, and when nothing was new', () => {
    expect(copyResultText({ added: ['a', 'b'], truncated: false })).toBe(
      'Muted 2 channels whose Twitch bell is off.',
    );
    expect(copyResultText({ added: ['a'], truncated: true })).toBe(
      'Muted 1 channel whose Twitch bell is off. Only your first 2,000 follows were checked.',
    );
    expect(copyResultText({ added: [], truncated: false })).toBe(
      'Nothing new to copy. Every channel whose Twitch bell is off is already muted.',
    );
  });

  it('names each refusal in plain words', () => {
    expect(copyErrorText('signed-out')).toBe('Sign in to Twitch to copy your bells.');
    expect(copyErrorText('wrong-account')).toContain('another account');
    expect(copyErrorText('stale')).toBe("That copy can't be undone anymore.");
    expect(copyErrorText('save-failed')).toBe("Couldn't save that. Try again.");
    expect(undoResultText(12)).toBe('Alerts are back on for 12 channels.');
    expect(undoResultText(0)).toBe('Nothing left to undo.');
  });
});
