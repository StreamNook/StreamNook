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
    expect(lib).toContain('apply_loaded_settings(&settings);');
  });

  it('feeds them again from the real file in the phone reload', () => {
    const reload = lib.indexOf('services::push_alerts::server::seed(&loaded);');
    expect(reload).toBeGreaterThan(-1);
    const fed = lib.indexOf('apply_loaded_settings(&loaded);', reload);
    expect(fed).toBeGreaterThan(reload);
    expect(fed - reload).toBeLessThan(200);
  });
});
