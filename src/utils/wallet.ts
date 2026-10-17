// What the channel wallet holds, from Rust's channel_state answers.
import type { ChannelBitsFace } from '../types';

/** The bits face when it is worth showing: bits on here. */
export const bitsOn = (bits: ChannelBitsFace | null | undefined): ChannelBitsFace | null => (bits?.enabled ? bits : null);

/** Whether the channel holds anything to spend: points (`points` is null
 *  where they are off) or bits. */
export const walletHasAny = (points: object | null, bits: ChannelBitsFace | null | undefined) =>
  points !== null || bitsOn(bits) !== null;
