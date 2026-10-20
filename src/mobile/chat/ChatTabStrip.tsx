// Chat room tabs. Under the player the strip hides while only the watched
// stream's chat is open, so the single-chat case keeps every pixel of height it
// has now; the Chats screen always shows it, with its controls at the end.
import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { X, Broadcast } from 'phosphor-react';
import { useChatTabsStore } from './chatTabsStore';
import { ProviderMark } from '../../components/ProviderLogo';
import { pickFolding, TAB_GAP, TAB_GAP_TIGHT, type FoldTab, type Folding } from './tabFolding';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';

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
const ChatTabStripImpl: React.FC<{
  /** Show even a single room (the Chats screen's header). */
  always?: boolean;
  /** Controls after the rooms, outside their glass (the Chats screen's play
   *  and add buttons). */
  trailing?: React.ReactNode;
  /** The bar's height. The Chats screen's header is taller, so its 40 px
   *  capsules get air above the separator instead of sitting on it. */
  height?: number;
  /** What sits in the left side, opposite the controls (the Chats screen's
   *  Shared Chat or Together group for the room on screen). */
  leading?: React.ReactNode;
  /** What sits at the row's very left edge, outside the centered group (the
   *  Chats screen's back button). */
  edge?: React.ReactNode;
}> = ({ always = false, trailing, height = CHAT_TAB_STRIP_H, leading, edge }) => {
  const tabs = useChatTabsStore((s) => s.tabs);
  // Platform marks only earn their place when the open rooms span platforms;
  // with every tab on Kick (or every tab on Twitch) they say nothing.
  const mixedPlatforms = new Set(tabs.map((t) => t.provider)).size > 1;
  const activeChannel = useChatTabsStore((s) => s.activeChannel);
  const setActive = useChatTabsStore((s) => s.setActive);
  const removeTab = useChatTabsStore((s) => s.removeTab);

  // The iPhone strip's layout (tabFolding.ts): the rooms and what sits beside
  // them are centered as one group, and when they outgrow the bar the row
  // compresses in place and never scrolls. Each room is measured once in each
  // form from a hidden copy; the row then takes the first form that fits.
  const groupRef = useRef<HTMLDivElement | null>(null);
  const leadingRef = useRef<HTMLDivElement | null>(null);
  const trailingRef = useRef<HTMLDivElement | null>(null);
  const measureRef = useRef<HTMLDivElement | null>(null);
  const [folding, setFolding] = useState<Folding>({ folded: new Set(), tight: false });
  const fit = useCallback(() => {
    const group = groupRef.current;
    const measure = measureRef.current;
    if (!group || !measure) return;
    const width = (el: Element | null | undefined) => (el ? el.getBoundingClientRect().width : 0);
    const sized: FoldTab[] = tabs.map((t) => {
      const of = (form: string) => width(measure.querySelector(`[data-room="${CSS.escape(t.channel)}"][data-form="${form}"]`));
      return { channel: t.channel, label: t.label, full: of('full'), uncapped: of('uncapped'), folded: of('folded'), foldedTight: of('tight') };
    });
    const beside = [leadingRef.current, trailingRef.current].filter((el) => el && el.childElementCount > 0);
    const available =
      group.clientWidth -
      beside.reduce((sum, el) => sum + width(el), 0) -
      GROUP_GAP * beside.length -
      CAPSULE_PADDING * 2;
    const next = pickFolding(sized, activeChannel, available);
    setFolding((prev) =>
      prev.tight === next.tight && prev.folded.size === next.folded.size && [...next.folded].every((c) => prev.folded.has(c))
        ? prev
        : next,
    );
  }, [tabs, activeChannel]);
  useLayoutEffect(fit, [fit]);
  useEffect(() => {
    const group = groupRef.current;
    if (!group) return;
    const observer = new ResizeObserver(() => fit());
    observer.observe(group);
    if (trailingRef.current) observer.observe(trailingRef.current);
    if (leadingRef.current) observer.observe(leadingRef.current);
    // The faces' and names' font may land after the first measure.
    if (measureRef.current) observer.observe(measureRef.current);
    return () => observer.disconnect();
  }, [fit]);

  if (tabs.length < (always ? 1 : 2)) return null;

  const crowded = folding.folded.size > 0;
  const pill = (tab: (typeof tabs)[number], form: { folded: boolean; crowded: boolean; tight: boolean }, live: boolean) => (
    <RoomPill
      key={tab.channel}
      tab={tab}
      active={tab.channel === activeChannel}
      mark={mixedPlatforms && tab.provider !== 'twitch'}
      {...form}
      onSelect={live ? () => setActive(tab.channel) : undefined}
      onClose={live ? () => removeTab(tab.channel) : undefined}
    />
  );

  return (
    <div
      className="relative shrink-0 flex items-center gap-1.5 px-2 border-b border-borderSubtle"
      style={{ height }}
    >
      {edge}
      {/* One centered group, so a single room never reads lopsided; past the
          width the rooms give way first. */}
      <div ref={groupRef} className="flex-1 min-w-0 flex items-center justify-center gap-1.5">
        <div ref={leadingRef} className="min-w-0 flex items-center empty:hidden">
          {leading}
        </div>
        {/* The row of rooms wears the glaze and the open room is the darker
            pill inside it. */}
        <div className="min-w-0 max-w-full chrome-glaze chrome-glaze--flat">
          <div
            className="flex items-center p-1 overflow-hidden rounded-full"
            style={{ gap: folding.tight ? TAB_GAP_TIGHT : TAB_GAP }}
          >
            {tabs.map((tab) =>
              pill(tab, { folded: folding.folded.has(tab.channel), crowded, tight: folding.tight }, true),
            )}
          </div>
        </div>
        <div ref={trailingRef} className="shrink-0 flex items-center gap-1.5 empty:hidden">
          {trailing}
        </div>
      </div>
      {/* Every room in every form, out of sight, for the measure. */}
      <div
        ref={measureRef}
        aria-hidden
        className="absolute left-0 top-0 invisible pointer-events-none flex flex-col items-start"
      >
        {tabs.map((tab) => (
          <React.Fragment key={tab.channel}>
            <div className="flex" data-room={tab.channel} data-form="full">
              {pill(tab, { folded: false, crowded: false, tight: false }, false)}
            </div>
            <div className="flex" data-room={tab.channel} data-form="uncapped">
              {pill(tab, { folded: false, crowded: true, tight: false }, false)}
            </div>
            <div className="flex" data-room={tab.channel} data-form="folded">
              {pill(tab, { folded: true, crowded: true, tight: false }, false)}
            </div>
            <div className="flex" data-room={tab.channel} data-form="tight">
              {pill(tab, { folded: true, crowded: true, tight: true }, false)}
            </div>
          </React.Fragment>
        ))}
      </div>
    </div>
  );
};

/** The gap between the group's parts, and the glass capsule's padding (p-1). */
const GROUP_GAP = 6;
const CAPSULE_PADDING = 4;

/**
 * One room, as the iPhone draws it (TabPill): its face, a broadcast mark on
 * the stream's own tab, its name, and a close button on every room but the
 * stream's. Folded (a crowded row), a room not on screen shows its face and
 * marks only. Offered less than its natural width, only the name gives way.
 */
const RoomPill: React.FC<{
  tab: ReturnType<typeof useChatTabsStore.getState>['tabs'][number];
  active: boolean;
  folded: boolean;
  /** Other rooms are folded: the room on screen may use the width they gave up. */
  crowded: boolean;
  /** The row is at its tightest: a folded room is a smaller face with less air. */
  tight: boolean;
  mark: boolean;
  onSelect?: () => void;
  onClose?: () => void;
}> = ({ tab, active, folded, crowded, tight, mark, onSelect, onClose }) => {
  const face = folded && tight ? 16 : 20;
  return (
    <div
      data-active-room={active ? 'true' : undefined}
      className={`flex items-center rounded-full h-8 ${active ? 'min-w-0 glaze-selected' : 'shrink-0'}`}
      style={{ paddingLeft: folded ? (tight ? 2 : 6) : 10, paddingRight: folded ? (tight ? 2 : 6) : 4 }}
    >
      <button
        onClick={onSelect}
        tabIndex={onSelect ? undefined : -1}
        className="min-w-0 flex items-center gap-1.5"
        style={{ paddingRight: folded ? 0 : 4 }}
        aria-label={folded ? tab.label : undefined}
      >
        <span className="relative shrink-0">
          {tab.avatar ? (
            <SevenTvAvatar
              // The tab's id is Twitch's alone; other platforms carry none.
              cosmeticsId={tab.provider === 'twitch' ? tab.channelId : null}
              px={face}
              src={tab.avatar}
              alt=""
              className="rounded-full object-cover shrink-0"
              style={{ width: face, height: face }}
              draggable={false}
            />
          ) : (
            <span
              className="rounded-full bg-surface flex items-center justify-center text-[10px] font-semibold text-textMuted shrink-0"
              style={{ width: face, height: face }}
            >
              {tab.label.charAt(0).toUpperCase()}
            </span>
          )}
          {/* Live rooms wear the sidebar's dot. The stream tab says it with
              its broadcast mark already. */}
          {tab.live && !tab.pinnedToStream && (
            <span
              aria-label="Live"
              className="absolute -right-0.5 -bottom-0.5 w-2 h-2 rounded-full bg-live ring-[1.5px] ring-background"
            />
          )}
        </span>
        {tab.pinnedToStream && <Broadcast size={11} weight="fill" className="text-accent shrink-0" />}
        {/* Two rooms can share a name across platforms; the mark tells them apart. */}
        {mark && <ProviderMark provider={tab.provider} size={11} />}
        {!folded && (
          // At most 96 px, unless the others folded for it.
          <span
            className={`min-w-0 text-[13px] truncate ${active ? 'text-textPrimary font-semibold' : 'text-textSecondary'}`}
            style={{ maxWidth: crowded && active ? undefined : 96 }}
          >
            {tab.label}
          </span>
        )}
        {/* Mentions of you here, until you read them. */}
        {tab.mentions > 0 && !active && (
          <span className="shrink-0 min-w-[16px] h-4 px-1 rounded-full bg-error text-white text-[10.5px] font-semibold leading-4 text-center tabular-nums">
            {tab.mentions > 99 ? '99+' : tab.mentions}
          </span>
        )}
      </button>
      {folded ? null : tab.pinnedToStream ? (
        // The stream tab follows the player and is not closable.
        <span className="w-0.5 shrink-0" />
      ) : (
        <button
          onClick={onClose}
          tabIndex={onClose ? undefined : -1}
          className="shrink-0 w-5 h-8 flex items-center justify-center text-textMuted active:text-textPrimary"
          aria-label={`Close ${tab.label} chat`}
        >
          <X size={12} weight="bold" />
        </button>
      )}
    </div>
  );
};

export const ChatTabStrip = React.memo(ChatTabStripImpl);
