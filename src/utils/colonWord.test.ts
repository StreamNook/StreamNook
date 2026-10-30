// The phones' "Only after a colon" (Rust emote_match::is_colon_word) and the
// desktop's : list (emoteSearchTrigger) open on the same words. Rust's
// emote_match test reads the same fixture.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { emoteSearchTrigger } from './chatInputWord';

const fixture = JSON.parse(readFileSync(resolve(__dirname, 'colonWord.fixture.json'), 'utf8')) as {
  cases: { word: string; colon: boolean }[];
};

describe('colon words', () => {
  it('are the words the desktop : list opens on', () => {
    expect(fixture.cases.length).toBeGreaterThan(20);
    for (const { word, colon } of fixture.cases) {
      expect(emoteSearchTrigger(word, word.length) !== null, JSON.stringify(word)).toBe(colon);
    }
  });

  it('are checked in Rust before the fill runs', () => {
    const src = readFileSync(resolve(__dirname, '../../src-tauri/src/commands/emotes.rs'), 'utf8');
    const resolveAt = src.indexOf('emote_match::Request::resolve(');
    // The saved switch is read, and its answer decides, before the fill runs.
    const read = src.indexOf('emote_match::colon_only_of(&s)');
    const gate = src.indexOf('if !emote_match::fill_gate(profile.as_deref(), &query, colon_only) {');
    expect(read).toBeGreaterThan(-1);
    expect(gate).toBeGreaterThan(read);
    expect(gate).toBeLessThan(resolveAt);
  });
});
