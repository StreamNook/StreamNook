// Pinned messages, as a dropdown that opens DOWNWARD.
//
// ONE container that grows, not a bar plus a second card below it. The header
// row is the pin's identity and stays put; the body unrolls inside the same
// bordered, blurred box. Two stacked containers also meant the sender's name was
// printed twice, once in each.
//
// Downward on purpose: this replaced a thin line that opened a bottom sheet, and
// a sheet rising from the bottom of the screen to reveal something docked at the
// TOP of chat is the wrong mental model. Content should come out of the thing
// you tapped.
import React from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { CaretDown, PushPin } from 'phosphor-react';

export interface PinnedItem {
  id: string;
  message_id: string;
  message_text: string;
  sender_name: string;
  sender_color: string;
}

interface Props {
  pins: PinnedItem[];
  expanded: boolean;
  onToggle: () => void;
}

export const PinnedBanner: React.FC<Props> = ({ pins, expanded, onToggle }) => {
  if (pins.length === 0) return null;
  const [first, ...rest] = pins;

  return (
    // The single container. Everything below lives inside this one box.
    <div className="sn-popover pointer-events-auto w-full overflow-hidden">
      <button
        type="button"
        onClick={onToggle}
        className="w-full flex items-center gap-2 px-2.5 py-1.5 text-left active:opacity-80"
      >
        <PushPin size={12} weight="fill" className="text-accent shrink-0" />
        <span
          className="text-[12px] font-bold shrink-0 max-w-[38%] truncate"
          style={{ color: first.sender_color || undefined, textShadow: '0 1px 2px rgba(0,0,0,0.8)' }}
        >
          {first.sender_name}
        </span>
        {/* Collapsed shows a one-line preview. Expanded drops it, because the
            full text appears directly below and the header already carries the
            name. */}
        <span className="text-[12px] text-textPrimary/85 truncate min-w-0 flex-1">
          {expanded ? '' : first.message_text}
        </span>
        {pins.length > 1 && (
          <span className="text-[11px] font-semibold text-textSecondary shrink-0 tabular-nums">
            +{pins.length - 1}
          </span>
        )}
        <motion.span
          className="shrink-0 text-textSecondary"
          animate={{ rotate: expanded ? 180 : 0 }}
          transition={{ duration: 0.18, ease: [0.2, 0.8, 0.2, 1] }}
        >
          <CaretDown size={12} weight="bold" />
        </motion.span>
      </button>

      {/* Height animation so the body unrolls out of the header rather than
          appearing. The parent's overflow-hidden is what makes it read as one
          box growing downward. */}
      <AnimatePresence initial={false}>
        {expanded && (
          <motion.div
            key="body"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
            className="overflow-hidden"
          >
            <div className="px-2.5 pb-2 pt-0.5 flex flex-col gap-2">
              {/* First pin: text only. Its author is already the header. */}
              <div className="text-[13px] leading-relaxed text-textPrimary">
                {first.message_text}
              </div>
              {rest.length > 0 && <div className="h-px bg-borderSubtle" />}
              {rest.map((pin) => (
                <div key={pin.id} className="text-[13px] leading-relaxed">
                  <span
                    className="font-bold"
                    style={{
                      color: pin.sender_color || undefined,
                      textShadow: '0 1px 2px rgba(0,0,0,0.8)',
                    }}
                  >
                    {pin.sender_name}
                  </span>
                  <span className="text-textPrimary">: {pin.message_text}</span>
                </div>
              ))}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
};
