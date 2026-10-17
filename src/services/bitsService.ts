// Typed wrappers for the bits commands (src-tauri/src/commands/bits.rs). Rust
// owns the rules, the balance and the Twitch calls; the page only shows them.
import { invoke } from '@tauri-apps/api/core';

/** A typed message that cheers, held back for you to confirm. */
export interface CheerCheck {
  /** Bare lowercase login. */
  channel: string;
  channel_id: string;
  text: string;
  total: number;
  /** null when the twitch.tv session could not be read. */
  balance: number | null;
  /** Why it cannot go out as it is. Empty = ready to cheer. */
  problems: string[];
  /** The balance is what stands in the way. */
  needs_bits: boolean;
  /** Twitch's animated Cheer gem for this total. */
  icon_url: string | null;
  /** The message with its cheers taken out (Rust). */
  plain_text: string;
}

export interface CheerOutcome {
  sent: boolean;
  balance: number | null;
  problem: string | null;
  /** Twitch would send it anonymously without the message: continue or edit. */
  can_continue: boolean;
  needs_bits: boolean;
}

export interface CheermoteAmount {
  bits: number;
  url: string;
  color: string;
}

export interface CheermoteChoice {
  /** As Twitch writes it ("Cheer"); the inserted word is prefix + amount. */
  prefix: string;
  custom: boolean;
  tiers: CheermoteAmount[];
}

export interface BitsState {
  enabled: boolean;
  cheer_min: number;
  emote_min: number;
  /** The amounts one cheermote can carry here (from Rust): tiers below
   * pick_min can't be cheered alone, a typed amount stays in the range. */
  pick_min: number;
  pick_max: number;
  /** Ready-made amounts, ascending, starting at pick_min. */
  quick_amounts: number[];
  balance: number | null;
  cheermotes: CheermoteChoice[];
  /** Twitch's animated Cheer gems by amount, ascending (from Rust). */
  icons: CheermoteAmount[];
}

/** The tier Twitch draws for an amount of bits: the highest one it reaches. */
export function cheerTierFor(tiers: CheermoteAmount[], bits: number): CheermoteAmount | null {
  const n = Math.max(1, bits);
  let tier: CheermoteAmount | null = tiers[0] ?? null;
  for (const t of tiers) if (t.bits <= n) tier = t;
  return tier;
}

/** The gem Twitch draws for an amount of bits. */
export const bitsIcon = (icons: CheermoteAmount[], bits: number): string | null =>
  cheerTierFor(icons, bits)?.url ?? null;

export const getBitsState = (channelLogin: string, channelId: string) =>
  invoke<BitsState>('get_bits_state', { channelLogin, channelId });

export const checkCheer = (channelLogin: string, channelId: string, text: string) =>
  invoke<CheerCheck | null>('check_cheer', { channelLogin, channelId, text });

export const sendCheer = (channelLogin: string, channelId: string, text: string, continueAnyway = false) =>
  invoke<CheerOutcome>('send_cheer', { channelLogin, channelId, text, continueAnyway });

export const openBitsWindow = () => invoke<string>('open_bits_window');

/** 12345 -> "12,345". */
export const formatBits = (n: number) => n.toLocaleString('en-US');
