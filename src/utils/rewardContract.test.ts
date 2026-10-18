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

  it('every redeem_reward invoke names only Rust parameters, the reward and channel always', () => {
    const rust = rustParams('src-tauri/src/commands/drops.rs', 'redeem_reward').map(camel);
    const src = read('src/components/ChannelPointsMenu.tsx');
    let at = src.indexOf(`'redeem_reward', {`);
    let calls = 0;
    while (at >= 0) {
      const end = src.indexOf('})', at);
      const keys = [...src.slice(at, end).matchAll(/^\s*([A-Za-z]+):/gm)].map((x) => x[1]);
      for (const k of keys) expect(rust, k).toContain(k);
      for (const k of ['channelLogin', 'channelId', 'rewardId']) expect(keys, k).toContain(k);
      calls += 1;
      at = src.indexOf(`'redeem_reward', {`, end);
    }
    // The detail view, the modify-emote picker and the choose-emote picker.
    expect(calls).toBe(3);
  });

  it('the menu no longer routes or mints transaction ids itself', () => {
    const src = read('src/components/ChannelPointsMenu.tsx');
    for (const gone of ['send_highlighted_message', 'send_sub_mode_message', 'redeem_channel_reward', 'unlock_chosen_emote', 'unlock_modified_emote', 'transactionId']) {
      expect(src, gone).not.toContain(gone);
    }
  });

  it('the built-in message rewards are named by the type Rust routes on', () => {
    const redeem = read('src-tauri/src/services/reward_redeem.rs');
    expect(redeem).toContain('Some(HIGHLIGHTED_MESSAGE_REWARD) => RewardKind::Highlight');
    expect(redeem).toContain('Some(SUB_MODE_BYPASS_REWARD) => RewardKind::SubMessage');
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
    for (const file of ['src/components/ChannelPointsMenu.tsx', 'src/utils/rewardStatus.ts', 'src-tauri/src/services/reward_redeem.rs']) {
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
