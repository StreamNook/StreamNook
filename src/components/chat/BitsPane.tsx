// The channel wallet's Bits tab: your balance, Twitch's Get Bits page, then
// two steps. Choose how many bits (a ready-made amount or any amount typed),
// then pick a cheermote, drawn at that amount, to put the word (Cheer250) in
// the message box. Sending it opens the cheer card, which asks before
// anything is spent. Rust (get_bits_state) decides what is cheerable here,
// the amount range, the ready-made amounts and the order; this only draws it.
import { useState } from 'react';
import { Tooltip } from '../ui/Tooltip';
import {
  bitsIcon,
  cheerTierFor,
  formatBits,
  openBitsWindow,
  type BitsState,
  type CheermoteChoice,
} from '../../services/bitsService';
import { Logger } from '../../utils/logger';
import { ACCENT_BUTTON, ACCENT_FILL, GLAZE_PILL } from '../ui/glazeButtons';

interface Props {
  state: BitsState;
  onInsert: (text: string) => void;
}

/** 1000 -> "1K", 250 -> "250": short enough for a row of amount chips. */
const shortBits = (n: number) => (n >= 1000 && n % 1000 === 0 ? `${n / 1000}K` : formatBits(n));

export function BitsPane({ state, onInsert }: Props) {
  const { pick_min: min, pick_max: max } = state;
  const [text, setText] = useState(String(min));
  const n = Number(text);
  const amount = Number.isInteger(n) && n >= min && n <= max ? n : null;

  // Enter puts an amount the field can't take back in range. Leaving the
  // field doesn't, so a click never adds an amount you didn't see.
  const settle = () => {
    if (amount !== null) return;
    setText(String(Number.isFinite(n) && text !== '' ? Math.min(max, Math.max(min, Math.trunc(n))) : min));
  };

  const channel = state.cheermotes.filter((c) => c.custom);
  const twitch = state.cheermotes.filter((c) => !c.custom);

  const grid = (list: CheermoteChoice[]) => (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(68px,1fr))] gap-1">
      {list.map((c) => {
        const tier = cheerTierFor(c.tiers, amount ?? min);
        return (
          <Tooltip key={c.prefix} content={amount !== null ? `Add ${c.prefix}${amount}` : ''} side="top">
            <button
              type="button"
              disabled={amount === null}
              onClick={() => amount !== null && onInsert(`${c.prefix}${amount}`)}
              className="group flex min-w-0 flex-col items-center gap-1 rounded-lg px-1 py-2 transition-colors hover:bg-white/[0.05] disabled:opacity-40 disabled:hover:bg-transparent"
            >
              {tier && <img src={tier.url} alt="" className="h-8 w-8 object-contain" />}
              <span className="w-full truncate text-center text-[10.5px] text-textSecondary group-hover:text-textPrimary">
                {c.prefix}
              </span>
            </button>
          </Tooltip>
        );
      })}
    </div>
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b border-white/[0.06] p-2 pl-3">
        {bitsIcon(state.icons, state.balance ?? 1) && (
          <img src={bitsIcon(state.icons, state.balance ?? 1)!} alt="" className="h-5 w-5 flex-shrink-0 object-contain" />
        )}
        <span className="min-w-0 truncate text-[13px] text-textPrimary">
          {state.balance !== null ? (
            <>
              <span className="font-semibold tabular-nums">{formatBits(state.balance)}</span> bits
            </>
          ) : (
            'Sign in to Twitch to see your bits'
          )}
        </span>
        <button
          type="button"
          onClick={() => void openBitsWindow().catch((err) => Logger.warn('[Bits] Get Bits failed:', err))}
          className={`${GLAZE_PILL} ml-auto h-6 flex-shrink-0 px-3 text-[12px]`}
        >
          Get bits
        </button>
      </div>
      {state.cheermotes.length === 0 ? (
        <div className="flex h-24 items-center justify-center text-xs text-textSecondary">
          This channel's cheermotes haven't loaded yet.
        </div>
      ) : (
        <>
          <div className="border-b border-white/[0.06] px-3 pb-2.5 pt-2.5">
            <div className="flex items-center gap-1.5">
              <label className="glass-input flex flex-shrink-0 cursor-text items-center gap-1 rounded-full py-1 pl-1.5 pr-2 focus-within:border-white/20">
                <img src={bitsIcon(state.icons, amount ?? min) ?? undefined} alt="" className="h-4 w-4 object-contain" />
                <input
                  type="number"
                  inputMode="numeric"
                  min={min}
                  max={max}
                  step={1}
                  value={text}
                  aria-label="Bits per cheer"
                  onFocus={(e) => e.currentTarget.select()}
                  onChange={(e) => setText(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      settle();
                    }
                  }}
                  className="w-[6.2ch] bg-transparent text-[12.5px] font-semibold tabular-nums text-textPrimary outline-none [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
                />
                <span className="text-[11px] text-textSecondary">bits</span>
              </label>
              <div className="flex min-w-0 flex-1 items-center justify-end gap-1">
                {state.quick_amounts.map((a) => (
                  <button
                    key={a}
                    type="button"
                    onClick={() => setText(String(a))}
                    className={`${
                      a === amount ? `${ACCENT_BUTTON} !text-[11px]` : `${GLAZE_PILL} text-[11px]`
                    } h-6 min-w-0 px-2 tabular-nums`}
                    style={a === amount ? ACCENT_FILL : undefined}
                  >
                    {shortBits(a)}
                  </button>
                ))}
              </div>
            </div>
            <p className={`mt-2 text-[11px] leading-snug ${amount === null ? 'text-warning' : 'text-textSecondary'}`}>
              {amount === null
                ? `Choose ${formatBits(min)} to ${formatBits(max)} bits.`
                : min > 1
                  ? `Cheers here start at ${formatBits(min)} bits. Pick a cheermote to add it; you'll confirm before anything is spent.`
                  : "Pick a cheermote to add it. You'll confirm before anything is spent."}
            </p>
          </div>
          <div className="flex-1 overflow-y-auto px-2 pb-2 pt-1.5 scrollbar-thin">
            {channel.length > 0 && (
              <>
                <div className="px-1.5 pb-1 pt-1 text-[10px] font-bold uppercase tracking-wider text-textSecondary">
                  This channel
                </div>
                {grid(channel)}
                <div className="px-1.5 pb-1 pt-2.5 text-[10px] font-bold uppercase tracking-wider text-textSecondary">
                  Twitch
                </div>
              </>
            )}
            {grid(twitch)}
          </div>
        </>
      )}
    </div>
  );
}
