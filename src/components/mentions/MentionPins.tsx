// Mentions pinned at the top of a chat (SN-284). The newest pin is one line in
// the Twitch pin's glass bar (the phones: their pinned banner's popover): who,
// what they said, "+N" for the others, and X. A press on the line opens the
// mention in this chat (the surface's `onOpen`); X dismisses it everywhere.
// "+N" opens the rest under it.
//
// While the pointer is on the pins (or the list is open) what they show holds
// still: a mention that lands or a pin whose time is up never slides another
// one under the cursor. The hold lets go when the pointer leaves, the window
// or page goes away, or the last line it holds is put away.

import { useEffect, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { ArrowBendUpLeft, At, X } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { useMentionPins } from '../../hooks/useMentionPins';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { ProviderLogo } from '../ProviderLogo';
import { Logger } from '../../utils/logger';
import type { MentionPin } from '../../types';
import type { ProviderId } from '../../types/providers';

const dismiss = (id: string, undo: () => void) => {
  invoke('dismiss_mention_pin', { id }).catch((err) => {
    Logger.warn('[MentionPins] dismiss failed:', err);
    undo();
  });
};

/** One pinned mention, as a line: an open button (icon, name, words) beside
 *  its "+N" and X. Shared by chat, the phones and the settings preview. */
export function MentionPinRow({
  pin,
  mark,
  more = 0,
  moreOpen = false,
  onOpen,
  onMore,
  onDismiss,
  phone = false,
}: {
  pin: MentionPin;
  /** The phones' look: the Twitch pin banner's popover and finger-size
   *  targets, beside the phone's own pinned banner. */
  phone?: boolean;
  /** Show the platform's logo (the surface mixes platforms). */
  mark?: boolean;
  /** How many other pins sit behind this one ("+N"). */
  more?: number;
  moreOpen?: boolean;
  onOpen?: () => void;
  onMore?: () => void;
  onDismiss?: () => void;
}) {
  const design = useAppStore((s) => s.settings.chat_design);
  const { entry } = pin;
  const reply = entry.kind === 'reply';
  const accent = reply ? (design?.reply_color ?? '#ff6b6b') : (design?.mention_color ?? '#ff4444');
  const Icon = reply ? ArrowBendUpLeft : At;
  const said = entry.removed ? 'a message a moderator deleted' : entry.text;
  const textSize = phone ? 'text-[12px]' : 'text-[13px]';
  return (
    <div
      className={
        phone
          ? 'sn-popover sn-popover--static w-full pointer-events-auto overflow-hidden flex items-center pr-0.5'
          : 'glass-flyout rounded-xl w-full max-w-sm pointer-events-auto overflow-hidden flex items-center !pl-0 !pr-1.5 !py-0'
      }
      style={{ boxShadow: `inset 3px 0 0 ${accent}` }}
      data-mention-pin={entry.id}
    >
      <button
        type="button"
        onClick={onOpen}
        aria-label={`${entry.sender_name || entry.sender_login} ${reply ? 'replied to you' : 'mentioned you'}: ${said}. Open it`}
        className={`flex min-w-0 flex-1 items-center text-left ${
          phone ? 'gap-2 pl-2.5 py-2.5 active:opacity-80' : 'gap-2.5 pl-3.5 py-2 cursor-pointer'
        }`}
      >
        <Icon size={phone ? 13 : 14} weight="bold" className="flex-shrink-0" style={{ color: accent }} aria-hidden />
        {mark && <ProviderLogo provider={entry.platform as ProviderId} size={12} className="flex-shrink-0" />}
        <span
          className={`${textSize} font-bold min-w-0 max-w-[45%] truncate shrink`}
          style={entry.sender_color ? { color: entry.sender_color } : undefined}
        >
          {entry.sender_name || entry.sender_login}
        </span>
        {pin.run > 1 && (
          <span className="text-[11px] font-semibold text-textMuted flex-shrink-0 tabular-nums -ml-1">×{pin.run}</span>
        )}
        {entry.removed ? (
          <span className={`${textSize} italic text-textMuted truncate min-w-0 flex-1`}>Message deleted by a moderator</span>
        ) : (
          <span className={`${textSize} text-textPrimary/85 truncate min-w-0 flex-1`}>{entry.text}</span>
        )}
      </button>
      {more > 0 && (
        <button
          type="button"
          // A tap here never takes the keyboard down mid-reply.
          onPointerDown={(e) => e.preventDefault()}
          onClick={onMore}
          aria-expanded={moreOpen}
          aria-label={moreOpen ? 'Hide the other pinned mentions' : `Show ${more} more pinned mentions`}
          className={`flex-shrink-0 rounded-md text-[11px] font-semibold tabular-nums text-textSecondary transition-colors hover:bg-white/10 hover:text-textPrimary ${
            phone ? 'h-9 px-2' : 'ml-1 px-1.5 py-1'
          }`}
        >
          +{more}
        </button>
      )}
      <button
        type="button"
        onPointerDown={(e) => e.preventDefault()}
        onClick={onDismiss}
        aria-label="Dismiss this pinned mention"
        className={`flex-shrink-0 flex items-center justify-center rounded-lg text-textSecondary/70 transition-colors hover:bg-white/10 hover:text-textPrimary ${
          phone ? 'w-9 h-9 active:text-textPrimary' : 'ml-0.5 w-7 h-7'
        }`}
      >
        <X size={13} weight="bold" />
      </button>
    </div>
  );
}

/**
 * The pins for a chat showing `rooms` (Rust room keys joined with `|`). Draws
 * nothing while Pin mentions is off or nothing is pinned. The parent places
 * it; it fills the parent's width up to the Twitch pin's.
 */
export function MentionPins({
  rooms,
  mixed = false,
  onOpen,
}: {
  rooms: string;
  /** The surface mixes platforms: each pin shows its platform's logo. */
  mixed?: boolean;
  /** Open this mention in the surface (read it, scroll to it or show it). */
  onOpen: (pin: MentionPin) => void;
}) {
  // Keyed by the rooms: another chat starts with no hold, list or hidden ids.
  return <MentionPinsView key={rooms} live={useMentionPins(rooms)} mixed={mixed} onOpen={onOpen} />;
}

/** The pins block for pins the parent already holds (a phone screen, which
 *  also needs to know whether any are up to show its banner column). Key it
 *  by the room it shows. */
export function MentionPinsView({
  live,
  mixed = false,
  phone = false,
  onOpen,
}: {
  live: MentionPin[];
  mixed?: boolean;
  phone?: boolean;
  onOpen: (pin: MentionPin) => void;
}) {
  const [frozen, setFrozen] = useState<MentionPin[] | null>(null);
  const [gone, setGone] = useState<ReadonlySet<string>>(() => new Set());
  const [listOpen, setListOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const release = () => {
    setFrozen(null);
    setListOpen(false);
  };
  const markInside = useOutsidePress(listOpen, [rootRef], release);
  // The window or the page going away lets go too (the pointer may never
  // "leave" a window that lost focus or a page that was hidden).
  useEffect(() => {
    if (!frozen) return;
    const letGo = () => {
      setFrozen(null);
      setListOpen(false);
    };
    const onVisibility = () => {
      if (document.visibilityState === 'hidden') letGo();
    };
    window.addEventListener('blur', letGo);
    document.addEventListener('visibilitychange', onVisibility);
    return () => {
      window.removeEventListener('blur', letGo);
      document.removeEventListener('visibilitychange', onVisibility);
    };
  }, [frozen]);
  const shown = (frozen ?? live).filter((p) => !gone.has(p.entry.id));
  // X: hidden here at once (Rust's next answer leaves it out for good), shown
  // again if Rust could not dismiss it. Ids Rust no longer pins are dropped
  // from the set as it grows. Putting away the last line it holds lets the
  // hold go: the block unmounts, so no pointerleave would ever come.
  const putAway = (pin: MentionPin) => {
    const id = pin.entry.id;
    setGone((prev) => {
      const next = new Set([...prev].filter((g) => live.some((p) => p.entry.id === g)));
      return next.add(id);
    });
    if (shown.length <= 1) release();
    dismiss(id, () =>
      setGone((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      }),
    );
  };
  // Opening: Rust reads it and drops its pin, and its answer takes the line
  // away; the hold lets go so that answer shows.
  const open = (pin: MentionPin) => {
    release();
    onOpen(pin);
  };
  if (shown.length === 0) return null;
  const [head, ...rest] = shown;
  return (
    <div
      ref={rootRef}
      onPointerDownCapture={markInside}
      onPointerEnter={() => setFrozen((f) => f ?? live)}
      onPointerLeave={() => {
        if (!listOpen) setFrozen(null);
      }}
      className="pointer-events-none flex w-full flex-col items-center gap-1"
    >
      {/* A new newest pin fades in where the last one was: one line, never
          two overlapping while one leaves. */}
      <motion.div
        key={head.entry.id}
        initial={{ opacity: 0, y: -6 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
        className="flex w-full justify-center"
      >
        <MentionPinRow
          pin={head}
          phone={phone}
          mark={mixed}
          more={rest.length}
          moreOpen={listOpen}
          onOpen={() => open(head)}
          onMore={() => {
            setFrozen((f) => f ?? live);
            setListOpen((o) => !o);
          }}
          onDismiss={() => putAway(head)}
        />
      </motion.div>
      {listOpen &&
        rest.map((pin) => (
          <MentionPinRow
            key={pin.entry.id}
            pin={pin}
            phone={phone}
            mark={mixed}
            onOpen={() => open(pin)}
            onDismiss={() => putAway(pin)}
          />
        ))}
    </div>
  );
}

export default MentionPins;
