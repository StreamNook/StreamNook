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
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { CaretDown, PushPin, PushPinSlash } from 'phosphor-react';
import { PinnedChatter } from '../../components/chat/PinnedChatter';
import { useChannelChatMeta } from '../../stores/chatConnectionStore';
import { usePinStore } from '../../stores/pinStore';
import { Logger } from '../../utils/logger';
import { useChatTabsStore } from '../chat/chatTabsStore';
import { isModeratorFrom } from '../chat/modActions';
import { StackCard } from '../../components/chat/ChatOverlayStack';
import { FadeScroller } from '../../components/chat/FadeScroller';
import { BANNER_STYLE, GLAZE_BANNER } from '../../components/ui/glazeButtons';
import { pinnedStackId } from '../../components/chat/PinnedBar';
import type { StackCardControl, StackKind } from '../../components/chat/chatTopStack';

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
  /** Chat > Pinned Messages > Style: `hidden` draws nothing while the pin is folded. */
  collapsedStyle?: 'bar' | 'hidden';
  /** The pin as the `channel_pin` card of the top-of-chat stack (SN-284): Rust
   *  says whether it is open or a strip, a tap goes through the stack (which
   *  writes the one open store back), and `expanded` is only the store's value. */
  stack?: StackCardControl;
  measure?: (kind: StackKind, form: 'strip' | 'open' | 'line', height: number) => void;
}

// Lifts an unpainted name off the glass; a 7TV paint brings its own shadow.
const NAME_SHADOW = { textShadow: 'var(--pin-name-shadow, 0 1px 2px rgba(0,0,0,0.8))' } as const;

// Unpin, as the desktop's pinned card offers it: moderators and the
// broadcaster only, read off your own badges in the room on screen.
const UnpinButton: React.FC<{ broadcasterId: string; messageId: string }> = ({ broadcasterId, messageId }) => (
  <button
    type="button"
    onClick={async () => {
      try {
        await invoke('unpin_chat_message', { broadcasterId, messageId });
        usePinStore.getState().requestRefresh();
      } catch (err) {
        Logger.error('[PinnedBanner] Failed to unpin message:', err);
      }
    }}
    className="shrink-0 -my-1 -mr-1 p-1.5 rounded-lg text-textSecondary/70 active:text-red-400 active:bg-red-500/15 transition-colors"
    aria-label="Unpin message"
    title="Unpin message"
  >
    <PushPinSlash size={14} weight="bold" />
  </button>
);

export const PinnedBanner: React.FC<Props> = ({
  pins,
  expanded: storeOpen,
  onToggle: onStoreToggle,
  channelId,
  collapsedStyle = 'bar',
  stack,
  measure,
}) => {
  const onPresence = stack?.onPresence;
  const stackId = pinnedStackId(pins, storeOpen, collapsedStyle);
  React.useEffect(() => {
    onPresence?.(stackId);
  }, [onPresence, stackId]);
  React.useEffect(() => () => onPresence?.(null), [onPresence]);
  // In the stack Rust's form says whether it is open; a tap goes through it.
  const expanded = stack ? stack.placed && stack.form === 'open' : storeOpen;
  const onToggle = stack ? stack.onToggle : onStoreToggle;
  const activeRoom = useChatTabsStore((s) => {
    const t = s.tabs.find((tab) => tab.channel === s.activeChannel);
    return t?.provider === 'twitch' ? t.channel : null;
  });
  const { userBadges } = useChannelChatMeta(activeRoom);
  const canUnpin = !!activeRoom && !!channelId && isModeratorFrom(userBadges);
  // Unrolled, it floats over the rows: a tap outside rolls it up.
  const rootRef = React.useRef<HTMLDivElement>(null);
  const markInside = useOutsidePress(expanded && pins.length > 0, [rootRef], onToggle);
  if (pins.length === 0) return null;
  const [first, ...rest] = pins;
  // Held to the height Rust gave it, the glass takes that as its max-height and its body scrolls.
  const cap = stack && stack.placed && stack.form === 'open' && stack.height > 0 ? stack.height : 0;

  // Hidden: nothing over chat while folded, as on the desktop. The composer
  // menu's "Pinned message" opens it (the desktop's is the pin in its header).
  if (!expanded && collapsedStyle === 'hidden') return null;
  if (stack && (!stack.placed || stack.form === 'folded')) return null;

  const box = (
    // The single container. Everything below lives inside this one box.
    <div
      ref={rootRef}
      onPointerDownCapture={markInside}
      className={`${GLAZE_BANNER} relative flex flex-col pointer-events-auto w-full overflow-hidden`}
      style={{ ...BANNER_STYLE, maxHeight: cap > 0 ? cap : undefined }}
    >
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
            className="flex min-h-0 flex-col overflow-hidden"
          >
            <FadeScroller className="px-2.5 pb-2 pt-0.5 flex flex-col gap-2">
              {/* First pin: text only. Its author is already the header. */}
              <div className="flex items-start gap-2">
                <div className="flex-1 min-w-0 text-[13px] leading-relaxed text-textPrimary">
                  {first.message_text}
                </div>
                {canUnpin && channelId && <UnpinButton broadcasterId={channelId} messageId={first.message_id} />}
              </div>
              {rest.length > 0 && <div className="h-px bg-borderSubtle" />}
              {rest.map((pin) => (
                <div key={pin.id} className="flex items-start gap-2">
                  <div className="flex-1 min-w-0 text-[13px] leading-relaxed">
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
                    <span className="text-textPrimary">: <bdi>{pin.message_text}</bdi></span>
                  </div>
                  {canUnpin && channelId && <UnpinButton broadcasterId={channelId} messageId={pin.message_id} />}
                </div>
              ))}
            </FadeScroller>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
  return stack ? (
    <StackCard kind="channel_pin" control={stack} measure={measure} scrollOpen>
      {box}
    </StackCard>
  ) : (
    box
  );
};
