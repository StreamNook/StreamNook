// Chat room tabs. Hidden entirely while only the watched stream's chat is open,
// so the single-chat case keeps every pixel of height it has now.
import React from 'react';
import { X, Broadcast } from 'phosphor-react';
import { useChatTabsStore } from './chatTabsStore';

/** Fixed so the chat header, which is absolutely positioned over the top of the
 *  chat column, can offset itself by exactly this much and stop covering the
 *  tabs. Keep in sync with the strip's own style below. */
export const CHAT_TAB_STRIP_H = 45;

/** Whether the strip is currently taking up space. */
export function useChatTabsVisible(): boolean {
  return useChatTabsStore((s) => s.tabs.length >= 2);
}

/**
 * True when the chat on screen belongs to the stream that is playing.
 *
 * Anything sourced from `currentStream` — hype train, drop progress, polls,
 * predictions — is about the STREAM, while the chat below it is about the active
 * TAB. Those are the same thing only while this returns true; otherwise showing
 * them over another room's chat attributes them to the wrong channel.
 */
/**
 * The numeric channel id of the chat currently on screen.
 *
 * Anything that is a property of the CHAT rather than of the stream — pinned
 * messages being the main one — has to key off this, not off
 * `currentStream.user_id`, or it shows one room's data under another's messages.
 */
export function useActiveChatChannelId(): string | null {
  return useChatTabsStore((s) => {
    if (!s.activeChannel) return null;
    return s.tabs.find((t) => t.channel === s.activeChannel)?.channelId ?? null;
  });
}

export function useViewingStreamChat(): boolean {
  return useChatTabsStore((s) => {
    if (!s.activeChannel) return false;
    const active = s.tabs.find((t) => t.channel === s.activeChannel);
    return !!active?.pinnedToStream;
  });
}

// Memoized below: this takes no props and subscribes to the tab store itself,
// so the parent re-rendering (which it does on every arriving chat message)
// tells it nothing it did not already know.
const ChatTabStripImpl: React.FC = () => {
  const tabs = useChatTabsStore((s) => s.tabs);
  const activeChannel = useChatTabsStore((s) => s.activeChannel);
  const setActive = useChatTabsStore((s) => s.setActive);
  const removeTab = useChatTabsStore((s) => s.removeTab);

  if (tabs.length < 2) return null;

  return (
    <div
      className="shrink-0 flex items-center gap-1 px-2 overflow-x-auto border-b border-borderSubtle"
      style={{ height: CHAT_TAB_STRIP_H }}
    >
      {tabs.map((tab) => {
        const active = tab.channel === activeChannel;
        return (
          <div
            key={tab.channel}
            className={`shrink-0 flex items-center rounded-full pl-2.5 pr-1 h-8 ${
              active ? 'glass-button-active' : 'glass-button-static'
            }`}
          >
            <button
              onClick={() => setActive(tab.channel)}
              className="flex items-center gap-1.5 pr-1 max-w-[136px]"
            >
              {tab.avatar ? (
                <img
                  src={tab.avatar}
                  alt=""
                  className="w-5 h-5 rounded-full object-cover shrink-0"
                  draggable={false}
                />
              ) : (
                <span className="w-5 h-5 rounded-full bg-surface flex items-center justify-center text-[10px] font-semibold text-textMuted shrink-0">
                  {tab.label.charAt(0).toUpperCase()}
                </span>
              )}
              {tab.pinnedToStream && (
                <Broadcast size={11} weight="fill" className="text-accent shrink-0" />
              )}
              <span
                className={`text-[13px] truncate ${
                  active ? 'text-textPrimary font-semibold' : 'text-textSecondary'
                }`}
              >
                {tab.label}
              </span>
            </button>
            {/* The stream tab follows the player and is not closable. */}
            {tab.pinnedToStream ? (
              <span className="w-1.5" />
            ) : (
              <button
                onClick={() => removeTab(tab.channel)}
                className="w-6 h-6 flex items-center justify-center text-textMuted active:text-textPrimary"
                aria-label={`Close ${tab.label} chat`}
              >
                <X size={12} weight="bold" />
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
};

export const ChatTabStrip = React.memo(ChatTabStripImpl);
