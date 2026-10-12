// The channel point rewards menu and Rust meet only through serialized
// strings and invoke argument names, so a rename on one side fails silently:
// an unknown `reason` renders as nothing special, and an invoke argument whose
// name no longer matches the Rust parameter is refused by Tauri at call time
// with no sign of it in the menu until someone presses Send.
//
// The copy checks cover the words the reward flow and the YouTube send path
// show to the viewer: no dash punctuation, and no twitch.tv detour.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { formatCostShort, newTransactionId, rewardKind, rewardTakesText } from './rewardStatus';

const root = resolve(__dirname, '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');

const snake = (s: string) => s.replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase();
const camel = (s: string) => s.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());

/** Rust's `RewardBlock` variants, as serde writes them (snake_case). */
function rustRewardBlocks(): string[] {
  const src = read('src-tauri/src/models/drops.rs');
  const m = src.match(/pub enum RewardBlock \{([\s\S]*?)\n\}/);
  if (!m) throw new Error('RewardBlock not found in models/drops.rs');
  return m[1]
    .split('\n')
    .map((l) => l.replace(/\/\/.*$/, '').trim())
    .filter((l) => /^[A-Z]\w*,?$/.test(l))
    .map((l) => snake(l.replace(',', '')))
    .sort();
}

/** The TS `RewardBlock` union members. */
function tsRewardBlocks(): string[] {
  const src = read('src/types/index.ts');
  const m = src.match(/export type RewardBlock =([\s\S]*?);/);
  if (!m) throw new Error('RewardBlock not found in types/index.ts');
  return [...m[1].matchAll(/'([a-z_]+)'/g)].map((x) => x[1]).sort();
}

/** A Tauri command's parameter names, from its Rust signature. */
function rustParams(file: string, fnName: string): string[] {
  const src = read(file);
  const m = src.match(new RegExp(`pub async fn ${fnName}\\(([\\s\\S]*?)\\)\\s*->`));
  if (!m) throw new Error(`${fnName} not found in ${file}`);
  return m[1]
    .split('\n')
    .map((l) => l.replace(/\/\/.*$/, '').trim())
    .join(' ')
    .split(',')
    .map((p) => p.trim())
    .filter(Boolean)
    .map((p) => p.split(':')[0].trim())
    .sort();
}

/** The argument keys of an `invoke('<command>', { ... })` call in a TS file. */
function invokeKeys(file: string, command: string): string[] {
  const src = read(file);
  const at = src.indexOf(`'${command}', {`);
  if (at < 0) throw new Error(`invoke('${command}') not found in ${file}`);
  const end = src.indexOf('})', at);
  return [...src.slice(at, end).matchAll(/^\s*([A-Za-z]+):/gm)].map((x) => x[1]).sort();
}

/** A `pub const NAME: &str = "value";` from a Rust file. */
function rustConst(file: string, name: string): string {
  const m = read(file).match(new RegExp(`pub const ${name}: &str = "([^"]+)"`));
  if (!m) throw new Error(`${name} not found in ${file}`);
  return m[1];
}

/** String literals in a Rust file, above its test module. */
function rustStrings(file: string, from?: string, to?: string): string[] {
  let src = read(file);
  const tests = src.indexOf('#[cfg(test)]');
  if (tests >= 0) src = src.slice(0, tests);
  if (from) {
    const a = src.indexOf(from);
    if (a < 0) throw new Error(`${from} not found in ${file}`);
    const b = to ? src.indexOf(to, a) : -1;
    src = src.slice(a, b > a ? b : undefined);
  }
  return [...src.matchAll(/"((?:[^"\\\n]|\\.)*)"/g)].map((x) => x[1]);
}

/** En and em dash, built from code points so this file carries neither. */
const DASHES = new RegExp(`[${String.fromCharCode(0x2013, 0x2014)}]`);

describe('rewards: Rust and the menu agree', () => {
  it('every reason Rust can send is one the menu knows', () => {
    expect(tsRewardBlocks()).toEqual(rustRewardBlocks());
  });

  it('the sub-only message invoke names every Rust parameter', () => {
    const rust = rustParams('src-tauri/src/commands/drops.rs', 'send_sub_mode_message').map(camel);
    expect(invokeKeys('src/components/ChannelPointsMenu.tsx', 'send_sub_mode_message')).toEqual(rust);
  });

  it('the highlighted message invoke names every Rust parameter', () => {
    const rust = rustParams('src-tauri/src/commands/drops.rs', 'send_highlighted_message').map(camel);
    expect(invokeKeys('src/components/ChannelPointsMenu.tsx', 'send_highlighted_message')).toEqual(rust);
  });

  it('the custom reward invoke names only Rust parameters, the typed answer included', () => {
    const rust = rustParams('src-tauri/src/commands/drops.rs', 'redeem_channel_reward').map(camel);
    const keys = invokeKeys('src/components/ChannelPointsMenu.tsx', 'redeem_channel_reward');
    for (const k of keys) expect(rust, k).toContain(k);
    // The answer rides a conditional spread, which the key reader skips.
    expect(rust).toContain('textInput');
    expect(read('src/components/ChannelPointsMenu.tsx')).toContain('{ textInput: text }');
  });

  it('every message and custom redeem carries the retry-safe transaction id', () => {
    for (const command of ['send_sub_mode_message', 'send_highlighted_message', 'redeem_channel_reward']) {
      expect(rustParams('src-tauri/src/commands/drops.rs', command), command).toContain('transaction_id');
      expect(invokeKeys('src/components/ChannelPointsMenu.tsx', command), command).toContain('transactionId');
    }
  });

  it('a transaction id has the form Rust keeps (16 to 64 letters and digits)', () => {
    const a = newTransactionId();
    expect(a).toMatch(/^[0-9a-f]{32}$/);
    expect(newTransactionId()).not.toBe(a);
  });

  it('the built-in message rewards route to their own senders by the type Rust writes', () => {
    const rewards = 'src-tauri/src/services/channel_rewards.rs';
    const highlight = rustConst(rewards, 'HIGHLIGHTED_MESSAGE_REWARD');
    const bypass = rustConst(rewards, 'SUB_MODE_BYPASS_REWARD');
    expect(rewardKind({ reward_type: highlight, is_user_input_required: true })).toBe('highlight');
    expect(rewardKind({ reward_type: bypass, is_user_input_required: true })).toBe('sub_message');
    expect(rewardTakesText('highlight')).toBe(true);
    expect(rewardTakesText('sub_message')).toBe(true);
  });
});

describe('rewards and YouTube send: viewer-facing copy', () => {
  it('Rust reward words carry no dash punctuation and no twitch.tv detour', () => {
    for (const s of rustStrings('src-tauri/src/services/channel_rewards.rs')) {
      expect(s, s).not.toMatch(DASHES);
      expect(s.toLowerCase(), s).not.toContain('twitch.tv');
    }
  });

  it('the rewards menu has no dash punctuation and no twitch.tv detour', () => {
    for (const file of ['src/components/ChannelPointsMenu.tsx', 'src/utils/rewardStatus.ts']) {
      const src = read(file);
      expect(src, file).not.toMatch(DASHES);
      expect(src.toLowerCase(), file).not.toContain('twitch.tv');
    }
  });

  it('YouTube send reasons carry no dash punctuation', () => {
    for (const s of rustStrings('src-tauri/src/services/providers/youtube_send.rs')) {
      expect(s, s).not.toMatch(DASHES);
    }
  });

  it('YouTubeProvider::send refusals carry no dash punctuation', () => {
    const strings = rustStrings(
      'src-tauri/src/services/providers/youtube.rs',
      'async fn send(&self',
      'async fn send_capability',
    );
    expect(strings.length).toBeGreaterThan(0);
    for (const s of strings) expect(s, s).not.toMatch(DASHES);
  });

  it('the reward redeem commands carry no dash punctuation', () => {
    const strings = rustStrings(
      'src-tauri/src/commands/drops.rs',
      'pub async fn redeem_channel_reward(',
      'pub async fn unlock_random_emote(',
    );
    expect(strings.length).toBeGreaterThan(0);
    for (const s of strings) expect(s, s).not.toMatch(DASHES);
  });
});

describe('formatCostShort edges', () => {
  it('a cost that rounds up to the next unit uses that unit (999,999 is 1M)', () => {
    expect(formatCostShort(999_999)).toBe('1M');
    expect(formatCostShort(999_500)).toBe('1M');
    expect(formatCostShort(999_499)).toBe('999k');
  });

  it('a rounded cost drops a trailing .0 (1,001 is 1k, 9,999 is 10k)', () => {
    expect(formatCostShort(1_001)).toBe('1k');
    expect(formatCostShort(9_999)).toBe('10k');
    expect(formatCostShort(99_999)).toBe('100k');
    expect(formatCostShort(1_050)).toBe('1.1k');
  });

  it('millions drop a trailing .0 too (1,999,999 is 2M)', () => {
    expect(formatCostShort(1_999_999)).toBe('2M');
    expect(formatCostShort(9_999_999)).toBe('10M');
    expect(formatCostShort(12_400_000)).toBe('12M');
  });
});
