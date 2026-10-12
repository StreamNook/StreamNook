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
import { PinnedChatter } from '../../components/chat/PinnedChatter';

export interface PinnedItem {
  id: string;
  message_id: string;
  message_text: string;
  sender_id?: string;
  sender_name: string;
  sender_login?: string;
  sender_shown_name?: string;
  sender_color: string;
  sender_badges?: Array<{ set_id: string; version: string }>;
}

interface Props {
  pins: PinnedItem[];
  expanded: boolean;
  onToggle: () => void;
  /** Broadcaster id, so subscriber and bits badges resolve to this channel. */
  channelId?: string;
}

// Lifts an unpainted name off the glass; a 7TV paint brings its own shadow.
const NAME_SHADOW = { textShadow: '0 1px 2px rgba(0,0,0,0.8)' } as const;

export const PinnedBanner: React.FC<Props> = ({ pins, expanded, onToggle, channelId }) => {
  if (pins.length === 0) return null;
  const [first, ...rest] = pins;

  return (
    // The single container. Everything below lives inside this one box.
    <div className="sn-popover sn-popover--static pointer-events-auto w-full overflow-hidden">
      <button
        type="button"
        onClick={onToggle}
        className="w-full flex items-center gap-2 px-2.5 py-1.5 text-left active:opacity-80"
      >
        <PushPin size={12} weight="fill" className="text-accent shrink-0" />
        {/* Collapsed: the name in its paint and at most the sender's role
            badge, so the name and preview keep the one line; the cap only
            stops a very long name from eating the preview. Expanded: this
            header is the open pin's author, so every badge shows as a chat
            row draws them, and badges that do not fit hide whole. */}
        <span className={`text-[12px] flex items-center gap-1 min-w-0 shrink ${expanded ? '' : 'max-w-[50%]'}`}>
          <PinnedChatter
            variant={expanded ? 'line' : 'bar'}
            provider="twitch"
            userId={first.sender_id ?? ''}
            name={first.sender_name}
            login={first.sender_login}
            shownName={first.sender_shown_name}
            color={first.sender_color}
            badges={first.sender_badges}
            channelId={channelId}
            nameClassName={expanded ? 'font-bold shrink-0 max-w-[8rem] truncate' : 'font-bold min-w-0 truncate'}
            plainNameStyle={NAME_SHADOW}
          />
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
                  <span className="inline-flex items-center gap-1 align-middle max-w-full">
                    <PinnedChatter
                      variant="line"
                      provider="twitch"
                      userId={pin.sender_id ?? ''}
                      name={pin.sender_name}
                      login={pin.sender_login}
                      shownName={pin.sender_shown_name}
                      color={pin.sender_color}
                      badges={pin.sender_badges}
                      channelId={channelId}
                      nameClassName="font-bold shrink-0"
                      plainNameStyle={NAME_SHADOW}
                    />
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
