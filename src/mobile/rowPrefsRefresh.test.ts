// The native chat rows (iPhone) draw link cards only when Rust stamped them,
// and Rust stamps them from the row prefs `chat_row_view::refresh` copies out
// of the settings. `ChatRules::refresh` returns early when the rule groups have
// not changed, so the row prefs have to be copied BEFORE that return: after it,
// a launch or a save that leaves the rules alone would keep the old prefs, and
// a viewer who turned link previews off would get the cards back.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const rules = readFileSync(resolve(__dirname, '../../src-tauri/src/services/chat_rules.rs'), 'utf8');

describe('native row prefs follow every settings refresh', () => {
  it('copies the row prefs before the unchanged-rules early return', () => {
    const start = rules.indexOf('pub fn refresh(settings: &Settings)');
    expect(start).toBeGreaterThan(-1);
    const copied = rules.indexOf('crate::services::chat_row_view::refresh(settings);', start);
    const early = rules.indexOf('RULES_HASH.load(', start);
    expect(copied).toBeGreaterThan(start);
    expect(early).toBeGreaterThan(copied);
  });
});
