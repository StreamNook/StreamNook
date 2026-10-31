// What the phone's emote sheet shows (SN-38), drawn from props only so it renders in tests: the
// emote large, its name, Rust's line under it, the stack's emotes when it has more than one, then
// who made it and the actions. Nothing above the answer moves when it lands: the line under the
// name keeps its height from the first frame, and everything that arrives with the answer sits
// below what was already shown.
import { useState } from 'react';
import { motion } from 'framer-motion';
import { Copy, ExternalLink } from 'lucide-react';

import { CreatorChip } from '../../components/chat/CreatorChip';
import type { EmoteCardTarget } from '../../stores/emoteCardStore';
import { emoteLargeUrl, type EmoteCredit, type EmoteCreator } from '../../services/emoteCredit';
import { openExternal } from '../../utils/openExternal';

/** Art box: the emote large enough to read, wide emotes keep their shape. */
const ART_PX = 112;

export interface EmoteSheetBodyProps {
  target: EmoteCardTarget;
  /** Every emote of the tapped stack the core can credit, in stack order (one for a plain emote). */
  stack: EmoteCardTarget[];
  /** Rust's answer (null until it lands, or when the lookup failed). */
  credit: EmoteCredit | null;
  /** The lookup has answered (or failed). */
  done: boolean;
  onClose: () => void;
  /** Shows another emote of the stack in place. */
  onSwitch: (target: EmoteCardTarget) => void;
  /** A Twitch creator with an account: the pane's profile sheet, once this sheet has gone. */
  onOpenTwitch: (creator: EmoteCreator) => void;
  onCopy: (name: string) => void;
}

export function EmoteSheetBody({ target, stack, credit, done, onClose, onSwitch, onOpenTwitch, onCopy }: EmoteSheetBodyProps) {
  const [art, setArt] = useState(() => emoteLargeUrl(target.provider, target.id, target.url));
  const creators = credit?.creators ?? [];
  const pageUrl = credit?.pageUrl;
  return (
    <div className="flex flex-col items-center pb-2 pt-2 text-center">
      <span className="flex items-center justify-center" style={{ height: ART_PX, maxWidth: '100%' }}>
        <img
          src={art}
          alt={target.name}
          draggable={false}
          referrerPolicy="no-referrer"
          className="max-h-full max-w-full object-contain drop-shadow-md"
          onError={() => {
            if (art !== target.url) setArt(target.url);
          }}
        />
      </span>
      <h2 className="mt-3 text-[18px] font-semibold leading-tight text-textPrimary [overflow-wrap:anywhere]">{target.name}</h2>
      {/* Held at its height from the first frame, so nothing moves when Rust's line lands. */}
      <p className="mt-1 min-h-[18px] text-[13px] leading-[18px] text-textSecondary">{done ? (credit?.subtitle ?? '') : ''}</p>
      {stack.length > 1 && (
        // Every emote of the stack, the one shown marked: a tap moves only the mark.
        <div className="mt-3 flex max-w-full gap-2 overflow-x-auto px-1 py-1" role="tablist" aria-label="Emotes stacked here">
          {stack.map((s) => {
            const on = s.provider === target.provider && s.id === target.id;
            return (
              <button
                key={`${s.provider}:${s.id}`}
                type="button"
                role="tab"
                aria-selected={on}
                aria-label={s.name}
                onClick={() => onSwitch(s)}
                className={`flex h-11 w-11 shrink-0 items-center justify-center rounded-lg ${on ? 'ring-2 ring-accent' : 'ring-1 ring-borderSubtle'}`}
              >
                <img src={s.url} alt="" draggable={false} referrerPolicy="no-referrer" className="max-h-8 max-w-8 object-contain" />
              </button>
            );
          })}
        </div>
      )}
      {!done ? (
        <p className="mt-4 text-[13px] text-textMuted">Looking up who made it...</p>
      ) : (
        <motion.div
          className="mt-4 flex w-full flex-col gap-3 text-left"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.15, ease: [0.2, 0.8, 0.2, 1] }}
        >
          {creators.map((c) => (
            <div key={`${c.role}:${c.name}`} className="flex flex-col gap-1">
              <span className="px-0.5 text-[12px] text-textMuted">{c.roleLabel}</span>
              <CreatorChip creator={c} onLeave={onClose} onOpenTwitch={onOpenTwitch} />
            </div>
          ))}
          {creators.length === 0 && credit?.emptyLine && (
            <p className="text-center text-[13px] text-textMuted">{credit.emptyLine}</p>
          )}
          <div className="mt-1 flex w-full gap-2">
            {pageUrl && credit?.pageLabel && (
              <button
                type="button"
                onClick={() => void openExternal(pageUrl)}
                className="flex min-h-[44px] flex-1 items-center justify-center gap-1.5 rounded-xl bg-glass px-3 text-[13px] font-medium text-textPrimary"
              >
                <ExternalLink size={15} />
                {credit.pageLabel}
              </button>
            )}
            <button
              type="button"
              onClick={() => onCopy(target.name)}
              className="flex min-h-[44px] flex-1 items-center justify-center gap-1.5 rounded-xl bg-glass px-3 text-[13px] font-medium text-textPrimary"
            >
              <Copy size={15} />
              Copy name
            </button>
          </div>
        </motion.div>
      )}
    </div>
  );
}
