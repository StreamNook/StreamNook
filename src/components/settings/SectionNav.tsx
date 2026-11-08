// Section rail for long settings tabs: the tab's sections listed down the
// right edge of the pane, a hairline track beside them, and one accent
// indicator that glides (spring) to whichever section you are reading.
// Clicking a name scrolls there smoothly and the indicator follows.
//
// Typography-led, no chips or boxes: it reads like the margin of a well set
// page and never competes with the settings themselves. Discovers sections
// from the DOM (every SettingsSection carries data-settings-section), so any
// tab with enough sections gets it without per-tab wiring.
//
// It follows whatever actually scrolls the sections: the container itself
// (the Settings dialog's pane), the nearest scrolling ancestor, or the window
// (streamnook.app's overlay page). A pinned header inside the container
// (data-settings-sticky) counts as the top edge, so the section under it is
// the one being read.

import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import { motion } from 'framer-motion';
import { Tooltip } from '../ui/Tooltip';
import { setSectionInView } from './sectionInView';

interface SectionNavProps {
  containerRef: RefObject<HTMLDivElement | null>;
  /** Re-discover sections when this changes (the active tab). */
  tabKey: string;
  hidden?: boolean;
  /** Fewer sections than this and the rail stays out of the way. */
  minSections?: number;
  /** Where the rail sticks, as a CSS length. Defaults to the pane's top inset. */
  stickyTop?: string;
  /** Tabs instead of a jump list: the host shows only the section picked here
   *  (streamnook.app's overlay page), so a click switches and nothing spies on
   *  the scroll. */
  selectedId?: string;
  onSelect?: (id: string) => void;
}

interface Entry {
  id: string;
  label: string;
}

/** The element whose scrolling moves `el`, or null for the window. */
function scrollerOf(el: HTMLElement): HTMLElement | null {
  for (let node: HTMLElement | null = el; node; node = node.parentElement) {
    const { overflowY } = getComputedStyle(node);
    if ((overflowY === 'auto' || overflowY === 'scroll' || overflowY === 'overlay') && node !== document.body && node !== document.documentElement) {
      return node;
    }
  }
  return null;
}

export default function SectionNav({ containerRef, tabKey, hidden = false, minSections = 4, stickyTop, selectedId, onSelect }: SectionNavProps) {
  const [entries, setEntries] = useState<Entry[]>([]);
  const [spiedId, setActiveId] = useState<string | null>(null);
  const tabs = onSelect != null;
  const activeId = tabs ? (selectedId ?? entries[0]?.id ?? null) : spiedId;
  // Tell a preview beside the settings which section is being read.
  const activeLabel = entries.find((e) => e.id === activeId)?.label ?? null;
  useEffect(() => {
    if (hidden) return;
    setSectionInView(activeLabel);
  }, [activeLabel, hidden]);
  const rafRef = useRef<number | null>(null);
  // While a click-driven scroll is in flight the spy would flicker through
  // every section on the way; pin the target until the scroll settles.
  const pinnedRef = useRef<{ id: string; until: number } | null>(null);

  const discover = useCallback(() => {
    const root = containerRef.current;
    if (!root) return;
    const found: Entry[] = [];
    root.querySelectorAll<HTMLElement>('section[data-settings-section]').forEach((el) => {
      const label = el.dataset.settingsSection || '';
      if (el.id && label) found.push({ id: el.id, label });
    });
    setEntries((prev) =>
      prev.length === found.length && prev.every((p, i) => p.id === found[i].id && p.label === found[i].label)
        ? prev
        : found,
    );
  }, [containerRef]);

  useEffect(() => {
    const root = containerRef.current;
    if (!root) return;
    discover();
    const mo = new MutationObserver(() => discover());
    mo.observe(root, { childList: true, subtree: true });
    return () => mo.disconnect();
  }, [containerRef, tabKey, discover]);

  /** The line a section has to pass to count as the one being read: the top
   *  of the scroll port, or the bottom of a pinned header inside it. The
   *  header is measured where it PINS (its sticky top plus its height), not
   *  where it sits right now: near the top of the page it has not pinned yet,
   *  and a jump aimed at its current bottom would stop short. */
  const readingLine = useCallback((scroller: HTMLElement | null): number => {
    const root = containerRef.current;
    const portTop = scroller ? scroller.getBoundingClientRect().top : 0;
    const sticky = root?.querySelector<HTMLElement>('[data-settings-sticky]');
    if (!sticky) return portTop;
    const stuckTop = parseFloat(getComputedStyle(sticky).top);
    return portTop + (Number.isFinite(stuckTop) ? stuckTop : 0) + sticky.offsetHeight;
  }, [containerRef]);

  useEffect(() => {
    const root = containerRef.current;
    if (!root || entries.length === 0 || tabs) return;
    const scroller = scrollerOf(root);
    const target: HTMLElement | Window = scroller ?? window;
    const measure = () => {
      rafRef.current = null;
      const pinned = pinnedRef.current;
      if (pinned && performance.now() < pinned.until) {
        setActiveId(pinned.id);
        return;
      }
      pinnedRef.current = null;
      const top = readingLine(scroller) + 24;
      let current: string | null = entries[0]?.id ?? null;
      for (const e of entries) {
        const el = document.getElementById(e.id);
        if (!el) continue;
        if (el.getBoundingClientRect().top <= top) current = e.id;
        else break;
      }
      const atBottom = scroller
        ? scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 2
        : window.scrollY + window.innerHeight >= document.documentElement.scrollHeight - 2;
      if (atBottom) current = entries[entries.length - 1].id;
      setActiveId(current);
    };
    const onScroll = () => {
      if (rafRef.current !== null) return;
      rafRef.current = requestAnimationFrame(measure);
    };
    measure();
    target.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      target.removeEventListener('scroll', onScroll);
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    };
  }, [containerRef, entries, tabKey, readingLine, tabs]);

  const jump = useCallback((id: string) => {
    const root = containerRef.current;
    if (!root) return;
    const scroller = scrollerOf(root);
    if (onSelect) {
      onSelect(id);
      // Scrolled down a long section, the next one would open somewhere above
      // the window: bring its top back under the pinned header, at once, since
      // the content was swapped rather than travelled through.
      requestAnimationFrame(() => {
        const top = document.getElementById(id)?.getBoundingClientRect().top;
        const line = readingLine(scroller) + 12;
        if (top != null && top < line) (scroller ?? window).scrollBy({ top: top - line });
      });
      return;
    }
    const el = document.getElementById(id);
    if (!el) return;
    // Pin for the length of the smooth scroll so the spy does not walk
    // through every section on the way.
    const now = performance.now();
    pinnedRef.current = { id, until: now + 700 };
    setActiveId(id);
    (scroller ?? window).scrollBy({
      top: el.getBoundingClientRect().top - readingLine(scroller) - 12,
      behavior: 'smooth',
    });
  }, [containerRef, readingLine, onSelect]);

  if (hidden || entries.length < minSections) return null;

  return (
    <nav
      aria-label={tabs ? 'Settings sections' : 'Sections on this page'}
      // As tabs the host decides when the rail fits; as a jump list it shows
      // from 1120 px.
      className={`sn-section-rail sticky top-6 w-[168px] flex-shrink-0 self-start ${tabs ? 'block' : 'hidden min-[1120px]:block'}`}
      style={stickyTop ? { top: stickyTop } : undefined}
    >
      <div className="relative pl-4">
        {/* The track: a hairline the indicator travels along. */}
        <span aria-hidden className="absolute bottom-1 left-0 top-1 w-px bg-white/[0.08]" />
        <ul className="flex flex-col">
          {entries.map((e) => {
            const active = e.id === activeId;
            return (
              <li key={e.id} className="relative">
                {active && (
                  <motion.span
                    layoutId="sn-section-rail-indicator"
                    aria-hidden
                    className="absolute -left-4 top-[calc(50%-9px)] h-[18px] w-[2px] rounded-full bg-accent"
                    style={{ boxShadow: '0 0 10px color-mix(in srgb, var(--color-accent) 55%, transparent)' }}
                    transition={{ type: 'spring', stiffness: 460, damping: 36, mass: 0.6 }}
                  />
                )}
                {/* Tabs have room for the whole name, so it never needs repeating. */}
                <Tooltip content={e.label} disabled={tabs}>
                  <button
                    type="button"
                    onClick={() => jump(e.id)}
                    aria-current={active ? (tabs ? 'page' : 'location') : undefined}
                    // As tabs it is the page's main navigation, so a step
                    // larger than the margin notes of a jump list.
                    className={`block w-full truncate text-left leading-snug transition-[color,transform] duration-200 ${
                      tabs ? 'py-[7px] text-[13px]' : 'py-[5px] text-[11.5px]'
                    } ${
                      active ? 'translate-x-0.5 font-medium text-textPrimary' : 'text-textMuted hover:text-textSecondary'
                    }`}
                  >
                    {e.label}
                  </button>
                </Tooltip>
              </li>
            );
          })}
        </ul>
      </div>
    </nav>
  );
}
