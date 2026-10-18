// The poll and prediction cards meet Rust only through serialized field names,
// event names and invoke argument names, so a rename on one side fails
// silently: a missing field draws as blank, and an argument name that no
// longer matches the Rust parameter is refused by Tauri at call time.
//
// The cards are views by design (Rust decides, TypeScript presents), so these
// checks also keep the old client-side decisions from creeping back in.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(__dirname, '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');

const camel = (s: string) => s.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());

const PREDICTION = 'src/components/PredictionOverlay.tsx';
const POLL = 'src/components/PollOverlay.tsx';

/** Field names of a Rust struct, as serde writes them (no renames in these files). */
function rustFields(file: string, name: string): string[] {
  const src = read(file);
  const m = src.match(new RegExp(`pub struct ${name} \\{([\\s\\S]*?)\\n\\}`));
  if (!m) throw new Error(`${name} not found in ${file}`);
  return [...m[1].matchAll(/^\s*pub (\w+):/gm)].map((x) => x[1]).sort();
}

/** Field names of a TS interface in types/index.ts. */
function tsFields(name: string): string[] {
  const src = read('src/types/index.ts');
  const m = src.match(new RegExp(`export interface ${name} \\{([\\s\\S]*?)\\n\\}`));
  if (!m) throw new Error(`${name} not found in types/index.ts`);
  return [...m[1].matchAll(/^ {2}(\w+)\??:/gm)].map((x) => x[1]).sort();
}

/** A Tauri command's parameter names, from its Rust signature. */
function rustParams(fnName: string): string[] {
  const src = read('src-tauri/src/commands/drops.rs');
  const m = src.match(new RegExp(`pub async fn ${fnName}\\(([\\s\\S]*?)\\)\\s*->`));
  if (!m) throw new Error(`${fnName} not found in commands/drops.rs`);
  return m[1]
    .split(',')
    .map((p) => p.trim())
    .filter(Boolean)
    .map((p) => camel(p.split(':')[0].trim()))
    .sort();
}

/** Every command a file invokes. */
const invoked = (src: string) =>
  [...new Set([...src.matchAll(/invoke(?:<[^>]*>)?\(\s*'([a-z_]+)'/g)].map((x) => x[1]))].sort();

/** Every event a file listens to. */
const listened = (src: string) =>
  [...new Set([...src.matchAll(/listen(?:<[^>]*>)?\(\s*'([a-z-]+)'/g)].map((x) => x[1]))].sort();

/** The argument keys of each call to `command` in `src`. */
function invokeKeys(src: string, command: string): string[][] {
  const calls: string[][] = [];
  const re = new RegExp(`invoke(?:<[^>]*>)?\\(\\s*'${command}',\\s*\\{([^}]*)\\}`, 'g');
  for (const m of src.matchAll(re)) {
    calls.push(
      m[1]
        .split(',')
        .map((p) => p.trim())
        .filter(Boolean)
        .map((p) => p.split(':')[0].trim())
        .sort(),
    );
  }
  return calls;
}

/** En and em dash, built from code points so this file carries neither. */
const DASHES = new RegExp(`[${String.fromCharCode(0x2013, 0x2014)}]`);

describe('poll and prediction cards: the TS types match Rust', () => {
  const structs: Array<[string, string]> = [
    ['src-tauri/src/services/predictions.rs', 'PredictionView'],
    ['src-tauri/src/services/predictions.rs', 'OutcomeView'],
    ['src-tauri/src/services/predictions.rs', 'ResultView'],
    ['src-tauri/src/services/predictions.rs', 'PredictionPush'],
    ['src-tauri/src/services/predictions.rs', 'BetView'],
    ['src-tauri/src/services/predictions.rs', 'BetPreset'],
    ['src-tauri/src/services/predictions.rs', 'PredictOutcome'],
    ['src-tauri/src/services/poll_view.rs', 'PollView'],
    ['src-tauri/src/services/poll_view.rs', 'ChoiceView'],
    ['src-tauri/src/services/poll_view.rs', 'PollPush'],
  ];
  for (const [file, name] of structs) {
    it(name, () => {
      expect(tsFields(name)).toEqual(rustFields(file, name));
    });
  }

  it('the push event names are the ones Rust emits', () => {
    expect(read('src-tauri/src/services/predictions.rs')).toContain('pub const EVENT: &str = "prediction-state";');
    expect(read('src-tauri/src/services/poll_view.rs')).toContain('pub const EVENT: &str = "poll-state";');
  });
});

describe('poll and prediction cards: only the Rust view API', () => {
  it('the prediction card invokes and listens to the view API only', () => {
    const src = read(PREDICTION);
    expect(invoked(src)).toEqual(['get_channel_prediction', 'predict', 'prediction_bet']);
    expect(listened(src)).toEqual(['prediction-state']);
  });

  it('the poll card invokes and listens to the view API only', () => {
    const src = read(POLL);
    expect(invoked(src)).toEqual(['vote_on_poll', 'watch_channel_poll_view']);
    expect(listened(src)).toEqual(['poll-state']);
  });

  it('every invoke names exactly the Rust parameters', () => {
    const cases: Array<[string, string]> = [
      [PREDICTION, 'get_channel_prediction'],
      [PREDICTION, 'prediction_bet'],
      [PREDICTION, 'predict'],
      [POLL, 'watch_channel_poll_view'],
      [POLL, 'vote_on_poll'],
    ];
    for (const [file, command] of cases) {
      const calls = invokeKeys(read(file), command);
      expect(calls.length, `${command} in ${file}`).toBeGreaterThan(0);
      for (const keys of calls) expect(keys, command).toEqual(rustParams(command));
    }
  });

  it('the cards no longer use the old events and commands', () => {
    const gone = [
      'prediction-created', 'prediction-updated', 'prediction-locked', 'prediction-ended',
      'prediction-self-updated', 'poll-created', 'poll-updated', 'poll-completed', 'poll-voted',
      'channel-points-spent', 'channel-points-earned', 'get_active_prediction',
      'get_channel_points_for_channel', 'get_channel_points_balance', 'place_prediction',
      "'watch_channel_poll'",
    ];
    for (const file of [PREDICTION, POLL]) {
      const src = read(file);
      for (const g of gone) expect(src, `${g} in ${file}`).not.toContain(g);
    }
  });

  it('the cards carry no decisions of their own', () => {
    // Percent math, winner picking, totals, number formatting and close
    // timers all belong to Rust now.
    const decisions = ['Math.round', '.reduce(', '.sort(', 'toLocaleString', 'winning_outcome', 'total_points', 'total_votes'];
    for (const file of [PREDICTION, POLL]) {
      const src = read(file);
      for (const d of decisions) expect(src, `${d} in ${file}`).not.toContain(d);
    }
    // The prediction card's only timer is the bet question's debounce.
    expect(read(POLL)).not.toContain('setTimeout');
    expect(read(PREDICTION).match(/setTimeout\(/g)?.length ?? 0).toBe(1);
  });

  it('the cards carry no dash punctuation', () => {
    for (const file of [PREDICTION, POLL, 'src/utils/deadlineClock.ts']) {
      expect(read(file), file).not.toMatch(DASHES);
    }
  });
});
