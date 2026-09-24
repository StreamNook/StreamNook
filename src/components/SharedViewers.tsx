import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Play, SquaresFour } from 'phosphor-react';
import type { Collaboration, Collaborator } from '../types';
import { useAppStore } from '../stores/AppStore';
import { usemultiNookStore } from '../stores/multiNookStore';
import { makeKey } from '../utils/providerKey';
import { collabLabel, groupWord } from '../utils/sharedViewers';
import { collabMissingFromGrid, watchCollabInMultiNook } from '../utils/collabMultiNook';
import { ACCENT_BUTTON, ACCENT_FILL } from './ui/glazeButtons';

// Twitch's Shared Viewership, presented as its own thing rather than folded
// into the viewer count: a "+2" beside the channel's name on cards and rows,
// a "Together" capsule in the chat header, both opening who the channel is
// streaming with. Rust decides who is in the group (active and live members
// only), orders them (the watched channel first, then by their own viewers)
// and keeps the counts current; this only draws it and hands clicks on.

const STACK_MAX = 3;
const OPEN_DELAY_MS = 150;
const CLOSE_DELAY_MS = 140;
const POPOVER_WIDTH = 272;
const EDGE = 8;

function Avatar({ member, size }: { member: Collaborator; size: number }) {
  return member.avatar_url ? (
    <img
      src={member.avatar_url}
      alt=""
      draggable={false}
      decoding="async"
      loading="lazy"
      className="shrink-0 rounded-full object-cover"
      style={{ width: size, height: size }}
    />
  ) : (
    <span
      className="grid shrink-0 place-items-center rounded-full bg-white/10 text-[9px] font-semibold uppercase text-textSecondary"
      style={{ width: size, height: size }}
    >
      {member.display_name.slice(0, 1)}
    </span>
  );
}

/** Overlapping faces, capped with a "+N". By default everyone the channel is
 *  streaming with (not the channel itself); `everyone` includes it. `ringClass`
 *  cuts each face out of whatever surface the stack sits on. */
export function CollabAvatarStack({
  collab,
  size = 16,
  ringClass = 'ring-background',
  everyone = false,
}: {
  collab: Collaboration;
  size?: number;
  ringClass?: string;
  everyone?: boolean;
}) {
  const faces = everyone ? collab.members : collab.members.filter((m) => !m.is_self);
  const shown = faces.slice(0, STACK_MAX);
  const extra = faces.length - shown.length;
  return (
    <span className="flex items-center -space-x-1.5">
      {shown.map((m) => (
        <span key={m.user_id} className={`flex rounded-full ring-2 ${ringClass}`}>
          <Avatar member={m} size={size} />
        </span>
      ))}
      {extra > 0 && (
        <span
          className={`grid place-items-center rounded-full bg-white/15 px-1 text-[9px] font-semibold tabular-nums text-textPrimary ring-2 ${ringClass}`}
          style={{ height: size, minWidth: size }}
        >
          +{extra}
        </span>
      )}
    </span>
  );
}

/** The "+2" beside a channel's name: glass inside the card's glass, so the
 *  `glaze-inset` lighting and no frost of its own. */
const NAME_TAG =
  'glaze-inset inline-flex h-[18px] shrink-0 items-center rounded-full bg-white/[0.08] px-1.5 text-[11px] font-semibold leading-none tabular-nums text-textPrimary';

/** How many others, as the name tag reads: "+2". */
function othersCount(collab: Collaboration): number {
  return collab.members.filter((m) => !m.is_self).length;
}

/** The name tag where the row or card itself is the tap target (Sidebar rows,
 *  the phone's cards): the count, nothing to open. */
export function TogetherTag({ collab }: { collab: Collaboration }) {
  return (
    <span className={NAME_TAG} aria-label={collabLabel(collab)}>
      +{othersCount(collab)}
    </span>
  );
}

/** Where a member already is on screen: the solo player, or a MultiNook tile. */
type OnScreen = 'watching' | 'grid' | null;

function MemberRow({ member, onScreen, onOpen }: { member: Collaborator; onScreen: OnScreen; onOpen?: () => void }) {
  const body = (
    <>
      <Avatar member={member} size={26} />
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="truncate text-xs font-medium text-textPrimary">{member.display_name}</span>
          {member.is_leader && (
            <span className="shrink-0 rounded bg-accent/15 px-1 py-px text-[9px] font-semibold uppercase tracking-wide text-accent">
              Host
            </span>
          )}
        </span>
        {onScreen && (
          <span className="block text-[10px] text-textMuted">{onScreen === 'watching' ? 'Watching' : 'In MultiNook'}</span>
        )}
      </span>
      <span className="shrink-0 text-xs tabular-nums text-textSecondary">{member.viewer_count.toLocaleString()}</span>
      {onOpen && (
        <span className="glaze-inset grid h-5 w-5 shrink-0 place-items-center rounded-full bg-white/[0.10] text-textPrimary">
          <Play size={10} weight="fill" />
        </span>
      )}
    </>
  );
  const row = 'flex w-full items-center gap-2 rounded-md px-1.5 py-1.5 text-left';
  return onOpen ? (
    <button
      type="button"
      onClick={onOpen}
      aria-label={`Watch ${member.display_name}`}
      className={`${row} transition-colors hover:bg-white/[0.06] focus-visible:bg-white/[0.06] focus-visible:outline-none`}
    >
      {body}
    </button>
  ) : (
    <div className={row}>{body}</div>
  );
}

/** MultiNook actions under the list. An empty grid gets one button; a grid
 *  with tiles in it gets the choice between adding the group and replacing
 *  the grid with it. */
function MultiNookActions({ collab, onDone }: { collab: Collaboration; onDone: () => void }) {
  const slots = usemultiNookStore((s) => s.slots);
  const count = collab.members.length;
  const run = (mode: 'replace' | 'append') => {
    onDone();
    void watchCollabInMultiNook(collab, mode);
  };
  const primary = `${ACCENT_BUTTON} flex flex-1 items-center justify-center gap-2 whitespace-nowrap px-3 py-2 !text-xs`;
  const secondary =
    'glass-button-secondary !rounded-full flex items-center justify-center whitespace-nowrap px-3 py-2 text-xs font-semibold text-textPrimary';

  if (slots.length === 0) {
    return (
      <div className="mt-1.5 flex">
        <button type="button" onClick={() => run('replace')} className={primary} style={ACCENT_FILL}>
          <SquaresFour size={14} weight="bold" />
          Watch {groupWord(count)} in MultiNook
        </button>
      </div>
    );
  }
  const missing = collabMissingFromGrid(collab, slots);
  return (
    <div className="mt-1.5 flex gap-1.5">
      <button type="button" onClick={() => run('append')} className={primary} style={ACCENT_FILL}>
        <SquaresFour size={14} weight="bold" />
        {missing === 0
          ? 'Open MultiNook'
          : missing === count
            ? `Add ${groupWord(count)} to grid`
            : `Add ${missing} more to grid`}
      </button>
      <button
        type="button"
        onClick={() => run('replace')}
        className={secondary}
        aria-label={`Replace the MultiNook grid with ${groupWord(count)} streams`}
      >
        Replace grid
      </button>
    </div>
  );
}

/**
 * Who a channel is streaming with: the group's faces and combined count, each
 * member with their own count, a member's row switching to their stream
 * (`onOpenChannel`) and, where the window has a MultiNook grid
 * (`allowMultiNook`), the whole group into it. The popover and the phone's
 * sheet both draw this.
 */
export function TogetherPanel({
  collab,
  onOpenChannel,
  allowMultiNook = false,
  onDone,
}: {
  collab: Collaboration;
  onOpenChannel?: (login: string) => void;
  allowMultiNook?: boolean;
  onDone: () => void;
}) {
  // Labels follow what is actually playing, not which card or header opened
  // this: opened from a Home card, the card's own channel is just another
  // stream you can start. Both are frontend store state (the player and the
  // grid live there), so the comparison runs here.
  const watchingLogin = useAppStore((s) => (s.streamUrl ? s.currentStream?.user_login?.toLowerCase() : undefined));
  const slots = usemultiNookStore((s) => s.slots);
  const gridActive = usemultiNookStore((s) => s.isMultiNookActive);
  const inGrid = new Set(slots.map((sl) => makeKey(sl.provider ?? 'twitch', sl.channelLogin)));
  const onScreen = (m: Collaborator): OnScreen =>
    m.login.toLowerCase() === watchingLogin && !gridActive
      ? 'watching'
      : inGrid.has(makeKey('twitch', m.login))
        ? 'grid'
        : null;

  return (
    <>
      <div className="flex items-center gap-2.5 px-1.5 pb-2 pt-1">
        <CollabAvatarStack collab={collab} size={22} everyone />
        <span className="min-w-0">
          <span className="block text-xs font-semibold text-textPrimary">Streaming together</span>
          <span className="block text-[11px] tabular-nums text-textSecondary">
            {collab.shared_viewers.toLocaleString()} viewers combined
          </span>
        </span>
      </div>
      {collab.members.map((m) => (
        <MemberRow
          key={m.user_id}
          member={m}
          onScreen={onScreen(m)}
          onOpen={
            onOpenChannel && onScreen(m) !== 'watching'
              ? () => {
                  onDone();
                  onOpenChannel(m.login);
                }
              : undefined
          }
        />
      ))}
      {allowMultiNook && <MultiNookActions collab={collab} onDone={onDone} />}
    </>
  );
}

/**
 * The trigger that opens the panel on hover or click. `name` is the "+2" beside
 * a stream card's channel name (the card underneath stays clickable, so
 * nothing here reaches it); `header` is the "Together" capsule in the chat
 * header and the title bar, glass inside their glass.
 */
export function TogetherChip({
  collab,
  variant,
  onOpenChannel,
  allowMultiNook = false,
}: {
  collab: Collaboration;
  variant: 'name' | 'header';
  onOpenChannel?: (login: string) => void;
  allowMultiNook?: boolean;
}) {
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  const openTimer = useRef<number | undefined>(undefined);
  const closeTimer = useRef<number | undefined>(undefined);

  useEffect(
    () => () => {
      window.clearTimeout(openTimer.current);
      window.clearTimeout(closeTimer.current);
    },
    [],
  );
  useEffect(() => {
    if (!anchor) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setAnchor(null);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [anchor]);

  const openNow = (el: HTMLElement) => {
    window.clearTimeout(openTimer.current);
    window.clearTimeout(closeTimer.current);
    setAnchor(el.getBoundingClientRect());
  };
  // Hover intent: a pointer crossing a grid of cards must not flash a popover
  // over every card it passes.
  const openSoon = (el: HTMLElement) => {
    window.clearTimeout(closeTimer.current);
    window.clearTimeout(openTimer.current);
    openTimer.current = window.setTimeout(() => setAnchor(el.getBoundingClientRect()), OPEN_DELAY_MS);
  };
  const keepOpen = () => window.clearTimeout(closeTimer.current);
  const closeSoon = () => {
    window.clearTimeout(openTimer.current);
    window.clearTimeout(closeTimer.current);
    closeTimer.current = window.setTimeout(() => setAnchor(null), CLOSE_DELAY_MS);
  };
  const close = () => setAnchor(null);

  // Below the chip when it fits, above it when it would run off the bottom.
  const estimatedHeight = 64 + collab.members.length * 42 + (allowMultiNook ? 46 : 0);
  const placement = anchor
    ? {
        left: Math.min(Math.max(anchor.left, EDGE), window.innerWidth - POPOVER_WIDTH - EDGE),
        top:
          anchor.bottom + 6 + estimatedHeight > window.innerHeight - EDGE
            ? Math.max(EDGE, anchor.top - 6 - estimatedHeight)
            : anchor.bottom + 6,
      }
    : null;

  const lit = anchor ? ' bg-white/[0.14]' : '';
  const trigger =
    variant === 'name' ? (
      <span className={`${NAME_TAG} transition-colors hover:bg-white/[0.14]${lit}`}>+{othersCount(collab)}</span>
    ) : (
      <span
        className={`glaze-inset flex items-center gap-1.5 rounded-full bg-white/[0.08] py-0.5 pl-0.5 pr-2 text-xs text-textPrimary transition-colors hover:bg-white/[0.14]${lit}`}
      >
        <CollabAvatarStack collab={collab} size={16} />
        <span>Together</span>
      </span>
    );

  return (
    <>
      <button
        type="button"
        data-tauri-drag-region="false"
        aria-label={collabLabel(collab)}
        aria-expanded={anchor !== null}
        aria-haspopup="dialog"
        onMouseEnter={(e) => openSoon(e.currentTarget)}
        onMouseLeave={closeSoon}
        onClick={(e) => {
          // A stream card underneath would start the stream.
          e.stopPropagation();
          if (anchor) close();
          else openNow(e.currentTarget);
        }}
        onContextMenu={(e) => e.stopPropagation()}
        className="pointer-events-auto flex rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
      >
        {trigger}
      </button>
      {anchor &&
        placement &&
        createPortal(
          <div
            role="dialog"
            aria-label="Streaming together"
            onMouseEnter={keepOpen}
            onMouseLeave={closeSoon}
            // React carries events out of a portal to the component's parents,
            // so without this a click in here would also click the card.
            onClick={(e) => e.stopPropagation()}
            onContextMenu={(e) => e.stopPropagation()}
            onMouseDown={(e) => e.stopPropagation()}
            className="sn-popover fixed z-[300] p-1.5"
            style={{ ...placement, width: POPOVER_WIDTH }}
          >
            <TogetherPanel
              collab={collab}
              onOpenChannel={onOpenChannel}
              allowMultiNook={allowMultiNook}
              onDone={close}
            />
          </div>,
          document.body,
        )}
    </>
  );
}
