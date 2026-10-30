// A song's links on its page: "Listen on" pills for the track's own links,
// then a quieter "Search on" row of logos. Every press goes through
// openSongLink (utils/songLinks.ts), which opens the link in the music app on
// this computer when one is installed, else in the browser. On the desktop a
// pill's tooltip names that app ("Opens in Cider"), its right end is a QR
// segment that shows the link as a code to open on a phone, and when its
// service has two apps here (Apple Music and Cider) a small chevron offers
// the other one. Each search logo shows its own QR button under the pointer.
// Phones draw the pills alone.
import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { ChevronDown, Globe, QrCode } from 'lucide-react';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { IS_MOBILE } from '../../utils/platform';
import {
  openSongLink,
  SONG_LAYER_ATTR,
  songAppsFor,
  songLinkTip,
  useSongApps,
  type SongApp,
} from '../../utils/songLinks';
import type { SongProvider } from '../../utils/songId';
import { SocialLogo } from '../ui/SocialLogo';
import { Tooltip } from '../ui/Tooltip';
import { GLAZE_PILL } from '../ui/glazeButtons';
import { markOf, markSize } from './serviceMarks';
import { SongQrDialog } from './SongQrDialog';
import type { SongLinks } from './songLinks';
import { qrFits, type SongQrRequest } from './songQr';

/** The house easing. */
const EASE = [0.2, 0.8, 0.2, 1] as const;

/** A pill of parts: the glazed capsule of GLAZE_PILL, each part lit on its
 *  own under the pointer. */
const PILL_SHELL =
  'glaze-inset flex h-9 min-w-0 items-stretch overflow-hidden rounded-full bg-white/[0.08] font-semibold text-textPrimary';
/** One part of it. */
const PART =
  'flex items-center outline-none transition-[background-color,color,filter] duration-150 hover:bg-white/[0.06] focus-visible:bg-white/[0.1] active:brightness-95';

/** The app menu's width. */
const MENU_WIDTH = 224;

/** Where the app menu opens: under its pill, left-aligned, inside the window. */
interface MenuPlace {
  top: number;
  left: number;
}

function placeMenu(pill: HTMLElement): MenuPlace {
  const r = pill.getBoundingClientRect();
  return {
    top: Math.round(r.bottom + 6),
    left: Math.round(Math.max(8, Math.min(r.left, window.innerWidth - MENU_WIDTH - 8))),
  };
}

/** A service's logo, sized to read as large as the round ones. */
function Mark({ service, px }: { service: string; px: number }) {
  return (
    <span className="flex h-[17px] w-[17px] shrink-0 items-center justify-center" aria-hidden="true">
      <SocialLogo platform={markOf(service)} size={markSize(service, px)} />
    </span>
  );
}

/** The other ways to open a link whose service has two apps here: each app,
 *  then the browser. Portaled, fixed under its pill, closed by a press
 *  outside it, Escape, a scroll or resize, or anything chosen in it. */
function AppMenu({
  url,
  service,
  apps,
  at,
  toggle,
  onClose,
}: {
  url: string;
  service: string;
  apps: SongApp[];
  /** Where it opens, or null while closed. */
  at: MenuPlace | null;
  /** The chevron that opens it: a press on it is not outside. */
  toggle: RefObject<HTMLButtonElement | null>;
  onClose: () => void;
}) {
  const open = at !== null;
  const menuRef = useRef<HTMLDivElement>(null);
  const pressedInside = useOutsidePress(open, [menuRef, toggle], onClose);

  useEffect(() => {
    if (!open) return;
    menuRef.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus({ preventScroll: true });
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
      toggle.current?.focus({ preventScroll: true });
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', onClose);
    window.addEventListener('blur', onClose);
    // A scroll anywhere but inside the menu moves the pill out from under it.
    const onScroll = (e: Event) => {
      if (!(e.target instanceof Node) || !menuRef.current?.contains(e.target)) onClose();
    };
    window.addEventListener('scroll', onScroll, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', onClose);
      window.removeEventListener('blur', onClose);
      window.removeEventListener('scroll', onScroll, true);
    };
  }, [open, onClose, toggle]);

  // Up and Down walk the choices.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp' && e.key !== 'Home' && e.key !== 'End') return;
    const items = [...(menuRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
    if (!items.length) return;
    e.preventDefault();
    const now = items.indexOf(document.activeElement as HTMLElement);
    const next =
      e.key === 'Home'
        ? 0
        : e.key === 'End'
          ? items.length - 1
          : (now + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
    items[next].focus();
  };

  const choose = (how: { app?: string; browser?: boolean }) => {
    onClose();
    void openSongLink(url, how);
  };

  if (typeof document === 'undefined') return null;
  // The Motion setting is Off: it shows and goes at once.
  const off = document.documentElement.dataset.motion === 'off';
  const item =
    'flex h-9 w-full items-center gap-2.5 rounded-[6px] px-2.5 text-left text-[13px] font-medium text-textPrimary outline-none transition-colors duration-150 hover:bg-glass-hover focus-visible:bg-glass-hover';
  return createPortal(
    <AnimatePresence>
      {at && (
        <motion.div
          key="song-app-menu"
          ref={menuRef}
          {...{ [SONG_LAYER_ATTR]: '' }}
          role="menu"
          aria-label={`Open in ${service}`}
          onPointerDownCapture={pressedInside}
          onKeyDown={onKeyDown}
          className="glass-flyout no-live-blur fixed shadow-[0_16px_36px_-12px_rgba(0,0,0,0.6)]"
          style={{
            top: at.top,
            left: at.left,
            width: MENU_WIDTH,
            zIndex: 'calc(var(--sn-z-over-player-lift, 0) + 60)',
            transformOrigin: 'top left',
          }}
          initial={off ? false : { opacity: 0, scale: 0.96, y: -4 }}
          animate={{ opacity: 1, scale: 1, y: 0, transition: { duration: 0.16, ease: EASE } }}
          exit={{ opacity: 0, scale: 0.97, y: -3, transition: { duration: off ? 0 : 0.12, ease: EASE } }}
        >
          {apps.map((app) => (
            <button key={app.target} type="button" role="menuitem" onClick={() => choose({ app: app.target })} className={item}>
              <Mark service={service} px={16} />
              <span className="truncate">{`Open in ${app.app}`}</span>
            </button>
          ))}
          <div role="separator" className="mx-1.5 my-1 h-px bg-borderSubtle" />
          <button type="button" role="menuitem" onClick={() => choose({ browser: true })} className={item}>
            <span className="flex h-[17px] w-[17px] shrink-0 items-center justify-center" aria-hidden="true">
              <Globe size={15} className="text-textSecondary" />
            </span>
            <span className="truncate">Open in your browser</span>
          </button>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}

/** One "Listen on" pill on the desktop: the link (opening in its default
 *  app), the chevron for the other app when there are two, and the QR part. */
function ListenPill({
  link,
  apps,
  onQr,
}: {
  link: SongProvider;
  apps: SongApp[];
  onQr: (from: HTMLElement) => void;
}) {
  const shellRef = useRef<HTMLSpanElement>(null);
  const chevronRef = useRef<HTMLButtonElement>(null);
  const [menuAt, setMenuAt] = useState<MenuPlace | null>(null);
  const closeMenu = useCallback(() => setMenuAt(null), []);
  const several = apps.length > 1;
  // A link too long for any code has no QR part.
  const coded = qrFits(link.url);
  return (
    <span ref={shellRef} className={`${PILL_SHELL} text-[12.5px]`}>
      <Tooltip content={songLinkTip(link.url, apps)} side="top" describe>
        <button
          type="button"
          onClick={() => void openSongLink(link.url)}
          className={`${PART} min-w-0 gap-2 pl-2.5 ${several ? 'pr-1.5' : 'pr-3'}`}
        >
          <Mark service={link.name} px={17} />
          <span className="truncate">{link.name}</span>
        </button>
      </Tooltip>
      {several && (
        <Tooltip content="Choose the app" side="top" disabled={menuAt !== null}>
          <button
            ref={chevronRef}
            type="button"
            aria-label={`Choose the app for ${link.name}`}
            aria-haspopup="menu"
            aria-expanded={menuAt !== null}
            onClick={() => setMenuAt(menuAt || !shellRef.current ? null : placeMenu(shellRef.current))}
            className={`${PART} w-7 justify-center pr-0.5 text-textMuted hover:text-textPrimary`}
          >
            <ChevronDown
              size={14}
              strokeWidth={2.3}
              aria-hidden="true"
              className={`transition-transform duration-150 ${menuAt ? 'rotate-180' : ''}`}
            />
          </button>
        </Tooltip>
      )}
      {coded && (
        <>
          <span aria-hidden="true" className="my-2 w-px shrink-0 bg-white/[0.14]" />
          <Tooltip content="Open on your phone" side="top">
            <button
              type="button"
              aria-label={`Show a QR code for ${link.name}`}
              onClick={(e) => onQr(e.currentTarget)}
              className={`${PART} w-9 justify-center pr-0.5 text-textMuted hover:text-textPrimary`}
            >
              <QrCode size={15} aria-hidden="true" />
            </button>
          </Tooltip>
        </>
      )}
      {several && (
        <AppMenu url={link.url} service={link.name} apps={apps} at={menuAt} toggle={chevronRef} onClose={closeMenu} />
      )}
    </span>
  );
}

export interface SongLinkButtonsProps {
  links: SongLinks;
  title: string;
  artist: string;
  /** The phone's screen, which lays its pills out two to a row. */
  screen: boolean;
}

/** The pills and logos under "Listen on". */
export function SongLinkButtons({ links, title, artist, screen }: SongLinkButtonsProps) {
  const allApps = useSongApps();
  // Phones open links on themselves: no code to scan, no app to choose.
  const desk = !IS_MOBILE && !screen;
  const [qr, setQr] = useState<{ request: SongQrRequest; from: HTMLElement } | null>(null);
  const closeQr = useCallback(() => setQr(null), []);
  const showQr = (link: SongProvider, search: boolean, from: HTMLElement) =>
    setQr({ request: { url: link.url, service: link.name, search }, from });

  return (
    <>
      {links.listen.length > 0 &&
        (desk ? (
          <div className="flex flex-wrap gap-2">
            {links.listen.map((l) => (
              <ListenPill key={l.url} link={l} apps={songAppsFor(allApps, l.name)} onQr={(from) => showQr(l, false, from)} />
            ))}
          </div>
        ) : (
          // The phone lays its pills out two to a row, so three never wrap raggedly.
          <div className={screen ? 'grid grid-cols-2 gap-2' : 'flex flex-wrap gap-2'}>
            {links.listen.map((l) => (
              <Tooltip key={l.url} content={l.url} side="top" describe>
                <button
                  type="button"
                  onClick={() => void openSongLink(l.url)}
                  className={`${GLAZE_PILL} flex h-9 min-w-0 items-center gap-2 pl-2.5 pr-3.5 text-[12.5px] ${screen ? 'w-full' : ''}`}
                >
                  <Mark service={l.name} px={17} />
                  <span className="truncate">{l.name}</span>
                </button>
              </Tooltip>
            ))}
          </div>
        ))}
      {links.search.length > 0 && (
        // Search pages, a step quieter than the track's own links: logos
        // only, each named by its tooltip.
        <div className={`flex flex-wrap items-center gap-2 ${links.listen.length ? 'mt-3' : ''}`}>
          {links.listen.length > 0 && <span className="mr-1 text-[12px] font-medium text-textMuted">Search on</span>}
          {links.search.map((l) => (
            <span key={l.url} className="group/search flex items-center">
              <Tooltip content={`Search on ${l.name}`} side="top">
                <button
                  type="button"
                  aria-label={`Search on ${l.name}`}
                  onClick={() => void openSongLink(l.url)}
                  className={`${GLAZE_PILL} flex h-8 w-8 items-center justify-center`}
                >
                  <SocialLogo platform={markOf(l.name)} size={markSize(l.name, 15)} />
                </button>
              </Tooltip>
              {desk && qrFits(l.url) && (
                // Shown under the pointer or the keyboard; its room is kept,
                // so nothing beside it moves.
                <Tooltip content="Open this search on your phone" side="top">
                  <button
                    type="button"
                    aria-label={`Show a QR code for the ${l.name} search`}
                    onClick={(e) => showQr(l, true, e.currentTarget)}
                    className="flex h-8 w-6 items-center justify-center rounded-full text-textMuted opacity-0 outline-none transition-[opacity,color] duration-150 hover:text-textPrimary focus-visible:opacity-100 group-hover/search:opacity-100 group-focus-within/search:opacity-100"
                  >
                    <QrCode size={14} aria-hidden="true" />
                  </button>
                </Tooltip>
              )}
            </span>
          ))}
        </div>
      )}
      {desk && (
        <SongQrDialog
          request={qr?.request ?? null}
          title={title}
          artist={artist}
          apps={qr ? songAppsFor(allApps, qr.request.service) : []}
          from={qr?.from ?? null}
          onClose={closeQr}
          onOpen={(app) => {
            if (!qr) return;
            void openSongLink(qr.request.url, app ? { app: app.target } : { browser: true });
            closeQr();
          }}
        />
      )}
    </>
  );
}
