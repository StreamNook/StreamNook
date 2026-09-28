import React, { useEffect, useRef } from 'react';
import { motion } from 'framer-motion';
import { EmoteTabCandidate } from '../../utils/chatInputWord';
import { Tooltip } from '../ui/Tooltip';
import EmoteThumb from './EmoteThumb';
import EmoteProviderLogo from './EmoteProviderLogo';

interface EmoteAutocompleteProps {
  current: EmoteTabCandidate;
  backwards: EmoteTabCandidate[];
  forwards: EmoteTabCandidate[];
}

const Caret: React.FC<{ direction: 'left' | 'right' }> = ({ direction }) => (
  <svg
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2.5"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={{ transform: direction === 'left' ? 'rotate(180deg)' : undefined, opacity: 0.55 }}
  >
    <path d="M9 6l6 6-6 6" />
  </svg>
);

const CandidateThumb: React.FC<{ tok: EmoteTabCandidate; size: number }> = ({ tok, size }) => {
  const emote = tok.emote;
  if (!emote) {
    return (
      <div
        className="flex items-center justify-center px-2 text-textSecondary text-[11px] font-semibold"
        style={{ height: size }}
      >
        {tok.name}
      </div>
    );
  }
  return (
    <Tooltip content={emote.name}>
      <span className="inline-flex">
        <EmoteThumb emote={emote} size={size} />
      </span>
    </Tooltip>
  );
};

/**
 * Tab completion carousel. Renders backwards-matches, current match (highlighted),
 * and forwards-matches above the chat input. The current match already reflects
 * what was inserted into the textarea on this Tab press, so the carousel is
 * purely a preview of what comes next.
 */
const EmoteAutocomplete: React.FC<EmoteAutocompleteProps> = ({ current, backwards, forwards }) => {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (ref.current) ref.current.scrollLeft = 0;
  }, [current.name]);

  return (
    <motion.div
      initial={{ opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 4 }}
      transition={{ duration: 0.12, ease: 'easeOut' }}
      className="sn-popover absolute z-[60] left-0 right-0 mb-2"
      style={{
        bottom: '100%',
      }}
      ref={ref}
    >
      <div className="flex items-stretch gap-1.5 px-3 py-3 overflow-hidden">
        <div className="flex items-center gap-1.5 flex-1 justify-end min-w-0 opacity-70">
          {backwards.length > 0 && <Caret direction="left" />}
          {backwards.map((tok) => (
            <div key={`b-${tok.name}`} className="px-2 py-1.5 rounded-md flex items-center justify-center">
              <CandidateThumb tok={tok} size={40} />
            </div>
          ))}
        </div>

        <div
          className="px-3 py-1.5 rounded-md flex items-center justify-center bg-white/10 border border-white/15 shadow-[inset_0_0_0_1px_rgba(255,255,255,0.05)]"
        >
          <CandidateThumb tok={current} size={52} />
        </div>

        <div className="flex items-center gap-1.5 flex-1 min-w-0 opacity-70">
          {forwards.map((tok) => (
            <div key={`f-${tok.name}`} className="px-2 py-1.5 rounded-md flex items-center justify-center">
              <CandidateThumb tok={tok} size={40} />
            </div>
          ))}
          {forwards.length > 0 && <Caret direction="right" />}
        </div>
      </div>

      <div className="px-3.5 py-2 border-t border-white/5 flex items-center justify-between">
        <span className="text-[12px] font-semibold text-white/85 truncate pr-2">
          {current.name}
          {current.emote && (
            <span className="ml-2 inline-flex items-center gap-1 align-middle px-1.5 py-0.5 rounded bg-white/[0.05] text-[10px] font-normal leading-none text-white/55">
              <EmoteProviderLogo provider={current.emote.provider} tinted className="w-3 h-3" />
              {current.sourceDetail ?? current.emote.provider}
            </span>
          )}
          {current.chatter && (
            <span className="ml-1.5 text-[9px] uppercase tracking-wider text-white/40">user</span>
          )}
        </span>
        <span className="flex items-center gap-1 opacity-60">
          <kbd className="px-1.5 py-0.5 rounded bg-white/10 text-[9px] font-mono text-white tracking-widest border border-white/10 leading-none">TAB</kbd>
          <span className="text-[10px] text-white">cycle</span>
        </span>
      </div>
    </motion.div>
  );
};

export default EmoteAutocomplete;
