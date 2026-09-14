// The browsable command menu: the button left of the emote picker opens it.
// Where the slash autocomplete is a one-line-per-command completer for people
// who already know what they want, this is the comfortable version for
// finding out what exists. Every command shows its full description, the
// selected one unfolds with a longer explanation, who can run it, and
// ready-made examples you can click straight into the box.
//
// Two sizes, one state. Inline: a popover above the chat box with an icon
// rail and a grouped list; the selected row unfolds in place. Expanded: the
// same search, rail and list in a large centered dialog with a full detail
// pane on the right, for when the chat column is too narrow to read
// comfortably. "More room" in the header switches; the choice is remembered.
//
// Keyboard, from the search box in either size: arrows move, Enter inserts,
// Left/Right change category, Escape closes.
//
// Presenter only. The command list arrives already filtered by role from
// ChatWidget, which is where the roles are known.

import { useEffect, useMemo, useRef, useState, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import {
  CornerDownLeft,
  LayoutGrid,
  Lock,
  Maximize2,
  Megaphone,
  Minimize2,
  Radio,
  Search,
  Shield,
  SlidersHorizontal,
  Sparkles,
  Users,
  X,
  type LucideIcon,
} from 'lucide-react';
import type { CommandDefinition } from '../../utils/chatCommands';
import { renderCommandUsage } from './CommandAutocomplete';
import { Tooltip } from '../ui/Tooltip';

type Category = CommandDefinition['category'];
type RailKey = 'all' | Category;

const CATEGORY_ORDER: Category[] = ['Everyone', 'Moderator', 'Chat Flow', 'Engagement', 'Broadcaster', 'Custom'];

const RAIL: { key: RailKey; label: string; icon: LucideIcon; blurb: string }[] = [
  {
    key: 'all',
    label: 'All commands',
    icon: LayoutGrid,
    blurb: 'Every command, with the ones that need a role in this channel dimmed.',
  },
  { key: 'Everyone', label: 'Everyone', icon: Users, blurb: 'Commands any viewer can use.' },
  { key: 'Moderator', label: 'Moderator', icon: Shield, blurb: 'Bans, timeouts, pins and AutoMod terms.' },
  {
    key: 'Chat Flow',
    label: 'Chat modes',
    icon: SlidersHorizontal,
    blurb: 'Slow, followers-only, sub-only and emote-only modes.',
  },
  { key: 'Engagement', label: 'Engagement', icon: Megaphone, blurb: 'Announcements, shoutouts and polls.' },
  { key: 'Broadcaster', label: 'Broadcaster', icon: Radio, blurb: 'Raids, ads, markers, title and category.' },
  { key: 'Custom', label: 'Your commands', icon: Sparkles, blurb: 'Custom commands from Settings, Chat, Custom Commands.' },
];

const WHO: Record<Category, string> = {
  Everyone: 'Anyone can use this',
  Moderator: 'Moderators and the broadcaster',
  'Chat Flow': 'Moderators and the broadcaster',
  Engagement: 'Moderators and the broadcaster',
  Broadcaster: 'Broadcaster only',
  Custom: 'Your own command',
};

const EXPANDED_KEY = 'sn-command-menu-expanded';

const KBD = 'rounded border border-white/10 bg-white/10 px-1.5 py-0.5 font-mono text-[9px] leading-none tracking-widest text-white shadow-sm';

export interface CommandMenuProps {
  open: boolean;
  onClose: () => void;
  /** Already filtered to what this person can run here, custom commands included. */
  commands: CommandDefinition[];
  /** Insert the command name (`/name `) the same way the autocomplete does. */
  onPick: (command: CommandDefinition) => void;
  /** Insert a full example invocation as the message text. */
  onInsertText: (text: string) => void;
  /** The toggle button: a click on it must not count as "outside". */
  ignoreRef?: RefObject<HTMLElement | null>;
  /** Roles in this channel. Commands beyond them still show, dimmed and locked. */
  isModerator: boolean;
  isBroadcaster: boolean;
}

// What a locked command needs, in the words of the tag and the detail line.
function lockFor(cmd: CommandDefinition, isModerator: boolean, isBroadcaster: boolean): { tag: string; line: string } | null {
  if (cmd.category === 'Everyone' || cmd.category === 'Custom') return null;
  if (cmd.category === 'Broadcaster' && !isBroadcaster && !isModerator) {
    return { tag: 'Broadcaster', line: 'Needs the broadcaster in this channel' };
  }
  if (!isModerator && !isBroadcaster) return { tag: 'Mods', line: 'Needs moderator status in this channel' };
  return null;
}

function matches(cmd: CommandDefinition, q: string): boolean {
  if (!q) return true;
  const hay = [cmd.name, cmd.usage, cmd.description, cmd.details ?? '', ...(cmd.examples ?? [])]
    .join(' ')
    .toLowerCase();
  return q.split(/\s+/).every((tok) => hay.includes(tok));
}

function readExpanded(): boolean {
  try {
    return localStorage.getItem(EXPANDED_KEY) === '1';
  } catch {
    return false;
  }
}

function writeExpanded(v: boolean) {
  try {
    localStorage.setItem(EXPANDED_KEY, v ? '1' : '0');
  } catch {
    /* private mode, nothing to remember with */
  }
}

const CommandMenu = ({ open, ...rest }: CommandMenuProps) => (
  <AnimatePresence>{open && <CommandMenuPanel {...rest} />}</AnimatePresence>
);

// Mounted only while open, so every open starts with a clean query,
// category and selection.
const CommandMenuPanel = ({
  onClose,
  commands,
  onPick,
  onInsertText,
  ignoreRef,
  isModerator,
  isBroadcaster,
}: Omit<CommandMenuProps, 'open'>) => {
  const [query, setQuery] = useState('');
  const [rail, setRail] = useState<RailKey>('all');
  const [selected, setSelected] = useState(0);
  const [expanded, setExpanded] = useState(readExpanded);
  const panelRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const rowRefs = useRef<(HTMLDivElement | null)[]>([]);
  // Only keyboard moves scroll the list; a hover never yanks it around.
  const scrollPendingRef = useRef(false);

  const counts = useMemo(() => {
    const c = new Map<Category, number>();
    for (const cmd of commands) c.set(cmd.category, (c.get(cmd.category) ?? 0) + 1);
    return c;
  }, [commands]);
  const railItems = useMemo(() => RAIL.filter((r) => r.key === 'all' || (counts.get(r.key) ?? 0) > 0), [counts]);
  const q = query.trim().toLowerCase();
  // A search looks across every category; the rail only narrows a browse.
  const visible = useMemo(() => {
    const pool = q || rail === 'all' ? commands : commands.filter((c) => c.category === rail);
    return CATEGORY_ORDER.flatMap((cat) => pool.filter((c) => c.category === cat && matches(c, q)));
  }, [commands, rail, q]);

  useEffect(() => {
    const t = setTimeout(() => inputRef.current?.focus({ preventScroll: true }), 30);
    return () => clearTimeout(t);
  }, [expanded]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (panelRef.current?.contains(t)) return;
      if (ignoreRef?.current?.contains(t)) return;
      onClose();
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [onClose, ignoreRef]);

  useEffect(() => {
    if (!scrollPendingRef.current) return;
    scrollPendingRef.current = false;
    const row = rowRefs.current[selected];
    const list = listRef.current;
    if (!row || !list) return;
    const r = row.getBoundingClientRect();
    const c = list.getBoundingClientRect();
    const stickyHeader = 28;
    if (r.top < c.top + stickyHeader) list.scrollTop -= c.top + stickyHeader - r.top;
    else if (r.bottom > c.bottom) list.scrollTop += r.bottom - c.bottom;
  }, [selected]);

  const current = visible[Math.min(selected, Math.max(0, visible.length - 1))];

  const pickRail = (key: RailKey) => {
    setQuery('');
    setRail(key);
    setSelected(0);
    listRef.current?.scrollTo({ top: 0 });
    inputRef.current?.focus({ preventScroll: true });
  };

  const toggleExpanded = () => {
    setExpanded((v) => {
      writeExpanded(!v);
      return !v;
    });
  };

  const onKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      scrollPendingRef.current = true;
      setSelected((i) => Math.min(i + 1, visible.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      scrollPendingRef.current = true;
      setSelected((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (current && !lockFor(current, isModerator, isBroadcaster)) onPick(current);
    } else if ((e.key === 'ArrowLeft' || e.key === 'ArrowRight') && !query) {
      e.preventDefault();
      const idx = railItems.findIndex((r) => r.key === rail);
      const next = railItems[(idx + (e.key === 'ArrowRight' ? 1 : railItems.length - 1)) % railItems.length];
      pickRail(next.key);
    }
  };

  const railBlurb = RAIL.find((r) => r.key === rail)?.blurb ?? '';
  const grouped = q || rail === 'all';

  const searchBar = (
    <div className="relative z-10 flex items-center gap-2 border-b border-white/5 px-3 py-2.5">
      <Search size={14} className="flex-shrink-0 text-white/40" />
      <input
        ref={inputRef}
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setSelected(0);
        }}
        onKeyDown={onKey}
        placeholder="Search commands, or what you want to do"
        spellCheck={false}
        className="min-w-0 flex-1 bg-transparent text-[13px] text-textPrimary placeholder:text-white/30 focus:outline-none"
      />
      <Tooltip content={expanded ? 'Back to the chat box' : 'More room'} side="bottom">
        <button
          type="button"
          onClick={toggleExpanded}
          aria-label={expanded ? 'Shrink the command menu' : 'Expand the command menu'}
          className="flex h-6 w-6 items-center justify-center rounded-md text-white/45 transition-colors hover:bg-white/[0.06] hover:text-white/90"
        >
          {expanded ? <Minimize2 size={13} /> : <Maximize2 size={13} />}
        </button>
      </Tooltip>
      {expanded ? (
        <button
          type="button"
          onClick={onClose}
          aria-label="Close"
          className="flex h-6 w-6 items-center justify-center rounded-md text-white/45 transition-colors hover:bg-white/[0.06] hover:text-white/90"
        >
          <X size={14} />
        </button>
      ) : (
        <kbd className={KBD}>ESC</kbd>
      )}
    </div>
  );

  const railColumn = (
    <div
      className={`relative flex flex-shrink-0 flex-col border-r border-white/5 py-2 ${
        expanded ? 'w-44 gap-0.5 px-2' : 'w-11 items-center gap-1'
      }`}
    >
      {railItems.map((r) => {
        const active = !q && rail === r.key;
        const Icon = r.icon;
        const count = r.key === 'all' ? commands.length : (counts.get(r.key) ?? 0);
        const button = (
          <button
            key={r.key}
            type="button"
            aria-pressed={active}
            onClick={() => pickRail(r.key)}
            className={`relative flex items-center rounded-lg transition-colors ${
              expanded ? 'h-8 w-full gap-2.5 px-2.5 text-left' : 'h-8 w-8 justify-center'
            } ${active ? 'text-accent' : 'text-white/45 hover:bg-white/[0.05] hover:text-white/85'}`}
          >
            {active && (
              <motion.span
                layoutId="sn-command-rail-indicator"
                aria-hidden
                className={`absolute top-[calc(50%-9px)] h-[18px] w-[2px] rounded-full bg-accent ${
                  expanded ? '-left-2' : '-left-[7px]'
                }`}
                style={{ boxShadow: '0 0 10px color-mix(in srgb, var(--color-accent) 55%, transparent)' }}
                transition={{ type: 'spring', stiffness: 460, damping: 36, mass: 0.6 }}
              />
            )}
            <Icon size={16} className="flex-shrink-0" />
            {expanded && (
              <>
                <span className={`min-w-0 flex-1 truncate text-[12.5px] ${active ? 'text-textPrimary' : ''}`}>
                  {r.label}
                </span>
                <span className="text-[10.5px] tabular-nums text-white/35">{count}</span>
              </>
            )}
          </button>
        );
        return expanded ? (
          button
        ) : (
          <Tooltip key={r.key} content={r.key === 'all' ? r.label : `${r.label} · ${count}`} side="right">
            {button}
          </Tooltip>
        );
      })}
    </div>
  );

  const list = (
    <div
      ref={listRef}
      className={`custom-scrollbar min-w-0 overflow-y-auto px-2 pb-2 ${expanded ? 'w-[300px] flex-shrink-0 border-r border-white/5' : 'flex-1'}`}
    >
      {!q && <div className="px-2 pb-1 pt-2.5 text-[11px] leading-snug text-white/45">{railBlurb}</div>}
      {visible.length === 0 && (
        <div className="px-2 py-8 text-center text-[12px] text-white/45">
          Nothing matches. Try a word from what you want to do, like ban, slow or reminder.
        </div>
      )}
      {visible.map((cmd, index) => {
        const first = index === 0 || visible[index - 1].category !== cmd.category;
        const isSel = index === selected;
        const lock = lockFor(cmd, isModerator, isBroadcaster);
        return (
          <div key={`${cmd.category}:${cmd.name}`}>
            {first && grouped && (
              <div
                className="sticky top-0 z-[1] -mx-2 mb-0.5 mt-1.5 px-4 py-1 text-[10px] font-semibold uppercase tracking-[0.14em] text-white/40 backdrop-blur-md"
                // Rides the Glassiness slider like the panel under it: rows
                // scroll beneath this header, and at Glassiness 0 the blur is
                // stripped globally, so a fixed 88% would ghost them through.
                style={{
                  backgroundColor:
                    'color-mix(in srgb, var(--color-background) calc(88% + (1 - var(--glass-strength)) * 12%), transparent)',
                }}
              >
                {RAIL.find((r) => r.key === cmd.category)?.label ?? cmd.category}
              </div>
            )}
            <div
              ref={(el) => {
                rowRefs.current[index] = el;
              }}
              role="option"
              aria-selected={isSel}
              tabIndex={-1}
              onMouseEnter={() => setSelected(index)}
              onClick={() => (lock ? setSelected(index) : onPick(cmd))}
              className={`my-0.5 rounded-lg border px-3 py-2 transition-colors ${lock ? 'cursor-default' : 'cursor-pointer'} ${
                isSel
                  ? 'border-white/10 bg-white/[0.06] shadow-[inset_0_0_20px_rgba(255,255,255,0.02)]'
                  : 'border-transparent hover:bg-white/[0.03]'
              }`}
            >
              <div className="flex items-start justify-between gap-2">
                <span
                  className={`min-w-0 flex-1 whitespace-normal break-words text-[13px] font-semibold leading-5 ${
                    lock ? 'text-white/90 opacity-50' : 'text-white/90'
                  }`}
                >
                  {renderCommandUsage(cmd.usage)}
                </span>
                {lock && (
                  <span className="mt-0.5 inline-flex flex-shrink-0 items-center gap-1 rounded-md border border-amber-400/35 bg-amber-400/[0.06] px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider text-amber-200/80">
                    <Lock size={9} />
                    {lock.tag}
                  </span>
                )}
                {!lock && cmd.local && (
                  <span className="mt-0.5 flex-shrink-0 rounded-md bg-white/[0.06] px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider text-white/45">
                    StreamNook
                  </span>
                )}
              </div>
              <div
                className={`mt-0.5 text-[11.5px] leading-snug ${lock ? 'text-white/35' : 'text-textSecondary'} ${expanded ? 'line-clamp-2' : ''}`}
              >
                {cmd.description}
              </div>
              {!expanded && (
                <AnimatePresence initial={false}>
                  {isSel && (
                    <motion.div
                      key="more"
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: 'auto', opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      transition={{ duration: 0.16, ease: 'easeOut' }}
                      className="overflow-hidden"
                    >
                      <div className="mt-2 flex flex-col gap-2 border-t border-white/[0.06] pt-2">
                        {cmd.details && <div className="text-[11.5px] leading-snug text-white/70">{cmd.details}</div>}
                        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-white/45">
                          <span>{WHO[cmd.category]}</span>
                          <span>{cmd.local ? 'Runs in StreamNook only, nothing is sent to chat' : 'Sent to Twitch'}</span>
                        </div>
                        {cmd.examples && cmd.examples.length > 0 && (
                          <div className="flex flex-wrap gap-1.5">
                            {cmd.examples.map((ex) =>
                              lock ? (
                                <code
                                  key={ex}
                                  className="rounded-md border border-white/[0.06] px-2 py-1 font-mono text-[11px] text-white/45"
                                >
                                  {ex}
                                </code>
                              ) : (
                                <button
                                  key={ex}
                                  type="button"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    onInsertText(ex);
                                  }}
                                  className="glass-button rounded-md px-2 py-1 font-mono text-[11px] text-textPrimary"
                                  title="Put this in the chat box"
                                >
                                  {ex}
                                </button>
                              ),
                            )}
                          </div>
                        )}
                        <div className="flex items-center gap-1.5 text-[10.5px] text-white/40">
                          {lock ? <Lock size={11} className="text-amber-300/80" /> : <CornerDownLeft size={11} />}
                          <span>{lock ? lock.line : `Enter or click puts /${cmd.name} in the box`}</span>
                        </div>
                      </div>
                    </motion.div>
                  )}
                </AnimatePresence>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );

  const currentLock = current ? lockFor(current, isModerator, isBroadcaster) : null;
  const detail = current ? (
    <div className="custom-scrollbar flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto px-6 py-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-white/40">
            {RAIL.find((r) => r.key === current.category)?.label ?? current.category}
          </div>
          <div className="mt-1 whitespace-normal break-words text-[19px] font-semibold leading-7 text-textPrimary">
            {renderCommandUsage(current.usage)}
          </div>
        </div>
        {currentLock ? (
          <span className="inline-flex flex-shrink-0 items-center gap-1.5 rounded-lg border border-amber-400/35 bg-amber-400/[0.06] px-3 py-1.5 text-[11.5px] text-amber-200/80">
            <Lock size={12} />
            {currentLock.line}
          </span>
        ) : (
          <button
            type="button"
            onClick={() => onPick(current)}
            className="glass-button inline-flex flex-shrink-0 items-center gap-2 rounded-lg px-3 py-1.5 text-[12.5px] font-medium text-textPrimary"
          >
            <CornerDownLeft size={13} />
            Put /{current.name} in the box
          </button>
        )}
      </div>
      <p className="text-[13px] leading-relaxed text-white/85">{current.description}</p>
      {current.details && <p className="text-[12.5px] leading-relaxed text-white/65">{current.details}</p>}
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-[12px]">
        <dt className="text-white/40">Who can use it</dt>
        <dd className="text-white/80">{WHO[current.category]}</dd>
        <dt className="text-white/40">Where it runs</dt>
        <dd className="text-white/80">
          {current.local ? 'Inside StreamNook only. Nothing is sent to the chat server.' : 'Sent to Twitch as a chat command.'}
        </dd>
      </dl>
      {current.examples && current.examples.length > 0 && (
        <div className="flex flex-col gap-1.5">
          <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-white/40">
            {currentLock ? 'Examples' : 'Try one'}
          </div>
          {current.examples.map((ex) =>
            currentLock ? (
              <div key={ex} className="glass-tile flex items-center gap-3 px-3 py-2 opacity-70">
                <code className="min-w-0 flex-1 break-all font-mono text-[12px] text-white/60">{ex}</code>
              </div>
            ) : (
              <button
                key={ex}
                type="button"
                onClick={() => onInsertText(ex)}
                className="glass-tile flex items-center justify-between gap-3 px-3 py-2 text-left transition-colors hover:bg-white/[0.04]"
              >
                <code className="min-w-0 flex-1 break-all font-mono text-[12px] text-textPrimary">{ex}</code>
                <span className="flex-shrink-0 text-[10.5px] text-white/40">Insert</span>
              </button>
            ),
          )}
        </div>
      )}
    </div>
  ) : (
    <div className="flex flex-1 items-center justify-center px-6 text-center text-[12.5px] text-white/40">
      Pick a command on the left to read about it.
    </div>
  );

  const footer = (
    <div className="flex items-center justify-between gap-2 border-t border-white/5 px-3 py-2 text-[10.5px] text-white/40">
      <span className="tabular-nums">
        {visible.length} {visible.length === 1 ? 'command' : 'commands'}
        {q ? ' match' : ''}
      </span>
      <span className="flex flex-shrink-0 items-center gap-1">
        <kbd className={KBD}>↑↓</kbd>
        <kbd className={KBD}>←→</kbd>
        <kbd className={KBD}>ENTER</kbd>
        {expanded && <kbd className={KBD}>ESC</kbd>}
      </span>
    </div>
  );

  if (expanded) {
    return createPortal(
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.15 }}
        className="fixed inset-0 z-[200] flex items-center justify-center bg-black/45 p-6"
      >
        <motion.div
          ref={panelRef}
          role="dialog"
          aria-modal
          aria-label="Chat commands"
          initial={{ opacity: 0, y: 10, scale: 0.98 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 10, scale: 0.98 }}
          transition={{ duration: 0.18, ease: 'easeOut' }}
          className="sn-popover flex h-[min(680px,90vh)] w-[min(960px,96vw)] flex-col overflow-hidden"
        >
          {searchBar}
          <div className="flex min-h-0 flex-1">
            {railColumn}
            {list}
            {detail}
          </div>
          {footer}
        </motion.div>
      </motion.div>,
      document.body,
    );
  }

  return (
    <motion.div
      ref={panelRef}
      role="dialog"
      aria-label="Chat commands"
      initial={{ opacity: 0, y: 6, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 6, scale: 0.98 }}
      transition={{ duration: 0.15, ease: 'easeOut' }}
      className="sn-popover absolute bottom-full left-0 right-0 z-[60] mb-2 flex h-[560px] max-h-[calc(100vh-120px)] origin-bottom flex-col overflow-hidden"
    >
      {searchBar}
      <div className="flex min-h-0 flex-1">
        {railColumn}
        {list}
      </div>
      {footer}
    </motion.div>
  );
};

export default CommandMenu;
