// A phone reads settings.json twice at launch: once before its data folder
// resolves (the defaults) and again after. The second read has to feed the
// same Rust consumers as the first, or the chat rules, the composer's emote
// matching and the rest stay on the defaults until some save.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const lib = readFileSync(resolve(__dirname, '../../src-tauri/src/lib.rs'), 'utf8');

describe('phone launch settings', () => {
  it('feeds the launch consumers from the file at launch', () => {
    expect(lib).toMatch(/^\s*apply_loaded_settings\(&settings\);/m);
  });

  it('feeds every launch consumer', () => {
    const start = lib.indexOf('fn apply_loaded_settings(settings: &Settings) {');
    expect(start).toBeGreaterThan(-1);
    const body = lib.slice(start, lib.indexOf('\n}', start));
    for (const call of [
      'services::chat_rules::ChatRules::refresh(settings);',
      'services::streamer_mode::StreamerMode::refresh(settings);',
      'services::mention_ping::refresh(settings);',
      'services::mention_inbox::refresh(settings);',
      'services::mention_watch::client::seed(settings);',
      'services::whisper_inbox::refresh_settings(settings);',
      'services::hidden_channels::refresh(settings);',
    ]) {
      expect(body).toMatch(new RegExp(`^\\s*${call.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`, 'm'));
    }
  });

  it('feeds them again from the real file in the phone reload', () => {
    const reload = lib.indexOf('services::push_alerts::server::seed(&loaded);');
    expect(reload).toBeGreaterThan(-1);
    const fed = lib.search(/^\s*apply_loaded_settings\(&loaded\);/m);
    expect(fed).toBeGreaterThan(reload);
    expect(fed - reload).toBeLessThan(200);
  });
});
