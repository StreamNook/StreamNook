// A song link as a QR code, to open it on a phone: a dialog centered over the
// window (and over the player's full screen, lifted like every modal chat can
// open), on a dim backdrop with no blur, since it may sit over a playing
// stream. It pops in from the button that opened it. Under the code: the link
// with Copy link, then every way to open it on this computer (each music app
// installed for its service, and the browser). Closes on Escape (the same
// press leaves full screen alone), a press outside it, or Close, and gives
// the keyboard back to the button that opened it. The code is drawn from
// songQr.ts with the StreamNook logo at its center.
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { Check, Copy, Globe, Link2, X } from 'lucide-react';
import logoUrl from '../../assets/streamnook-logo-128.webp';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { useAppStore } from '../../stores/AppStore';
import { SONG_LAYER_ATTR, type SongApp } from '../../utils/songLinks';
import { SocialLogo } from '../ui/SocialLogo';
import { Tooltip } from '../ui/Tooltip';
import { GLAZE_PILL } from '../ui/glazeButtons';
import { markOf, markSize } from './serviceMarks';
import { QUIET_ZONE, qrHeading, qrModel, shortLink, type QrModel, type SongQrRequest } from './songQr';

/** The code's tile, quiet zone included. */
const QR_PX = 300;
/** The modules' near-black. */
const INK = '#111317';
/** The house easing for the fades. */
const EASE = [0.2, 0.8, 0.2, 1] as const;
/** The dialog's heading, in Inter like the panel's. */
const HEADING: CSSProperties = {
  fontFamily: '"Inter", var(--app-font, sans-serif)',
  fontWeight: 780,
  letterSpacing: '-0.02em',
};

/** The code, on a transparent ground (the tile is the white). */
function QrCode({ model }: { model: QrModel }) {
  const side = model.size + QUIET_ZONE * 2;
  const { knockout, logo } = model;
  return (
    <svg
      viewBox={`${-QUIET_ZONE} ${-QUIET_ZONE} ${side} ${side}`}
      width={QR_PX}
      height={QR_PX}
      role="img"
      aria-label="QR code"
      className="block"
    >
      <path d={model.modules} fill={INK} />
      <path d={model.finders} fill={INK} fillRule="evenodd" />
      {knockout.size > 0 && (
        <>
          <rect
            x={knockout.x}
            y={knockout.y}
            width={knockout.size}
            height={knockout.size}
            rx={knockout.size * 0.24}
            fill="#fff"
          />
          <image href={logoUrl} x={logo.x} y={logo.y} width={logo.size} height={logo.size} preserveAspectRatio="xMidYMid meet" />
        </>
      )}
    </svg>
  );
}

export interface SongQrCardProps {
  request: SongQrRequest;
  title: string;
  artist: string;
  /** The music apps on this computer for the link's service, its default first. */
  apps: readonly SongApp[];
  /** Ids for the heading, so the dialog is named by it. */
  headingId?: string;
  onClose: () => void;
  /** Open the link in an app, or in the browser (null). */
  onOpen: (app: SongApp | null) => void;
}

/** What the dialog holds. */
export function SongQrCard({ request, title, artist, apps, headingId, onClose, onOpen }: SongQrCardProps) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(t);
  }, [copied]);
  const copy = () => {
    navigator.clipboard
      .writeText(request.url)
      .then(() => setCopied(true))
      .catch(() => useAppStore.getState().addToast('Could not copy the link', 'error'));
  };
  const mark = markOf(request.service);
  // Null for a link too long for any code: the dialog still opens it and copies it.
  const model = useMemo(() => qrModel(request.url), [request.url]);

  return (
    <div className="flex flex-col">
      <div className="flex items-start gap-3.5">
        <span
          className="flex h-11 w-11 shrink-0 items-center justify-center rounded-[14px] bg-glass-hover"
          aria-hidden="true"
        >
          <SocialLogo platform={mark} size={markSize(request.service, 24)} />
        </span>
        <div className="flex min-w-0 flex-1 flex-col pt-px">
          <h2 id={headingId} className="text-[18px] leading-[23px] text-textPrimary [text-wrap:balance]" style={HEADING}>
            {qrHeading(request)}
          </h2>
          <p className="mt-1 truncate text-[13px] leading-[18px] text-textMuted">
            <span className="font-medium text-textSecondary">{title}</span>
            {artist && <span> · {artist}</span>}
          </p>
        </div>
        {/* On the heading's first line, a step quieter than the content. */}
        <Tooltip content="Close" side="bottom">
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="-mr-2 -mt-[3px] flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-textMuted transition-colors hover:text-textPrimary"
          >
            <X size={18} />
          </button>
        </Tooltip>
      </div>

      {model ? (
        <>
          <div
            className="mx-auto mt-5 overflow-hidden rounded-[20px] bg-white shadow-[0_14px_32px_-18px_rgba(0,0,0,0.75)] ring-1 ring-black/[0.06]"
            style={{ width: QR_PX, height: QR_PX }}
          >
            <QrCode model={model} />
          </div>
          <p className="mt-3 text-center text-[12.5px] leading-[17px] text-textMuted">
            Point your phone's camera at the code
          </p>
        </>
      ) : (
        <p className="glass-tile mt-5 flex h-20 items-center justify-center !rounded-[20px] px-6 text-center text-[13px] leading-[18px] text-textMuted">
          This link is too long for a QR code
        </p>
      )}

      <div className="glass-tile mt-5 flex h-11 items-center gap-2.5 !rounded-xl pl-3.5 pr-1.5">
        <Link2 size={14} className="shrink-0 text-textMuted" aria-hidden="true" />
        <Tooltip content={request.url} side="top" describe>
          <span className="min-w-0 flex-1 truncate text-[12.5px] leading-[17px] text-textSecondary">
            {shortLink(request.url)}
          </span>
        </Tooltip>
        <button
          type="button"
          onClick={copy}
          className={`${GLAZE_PILL} flex h-8 shrink-0 items-center gap-1.5 px-3 text-[12px]`}
        >
          {copied ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
          {/* Both words hold the space, so the button never changes size;
              the shorter one keeps to its icon. */}
          <span className="grid text-left">
            <span className={`col-start-1 row-start-1 ${copied ? 'invisible' : ''}`}>Copy link</span>
            <span className={`col-start-1 row-start-1 ${copied ? '' : 'invisible'}`} aria-live="polite">
              {copied ? 'Copied' : ''}
            </span>
          </span>
        </button>
      </div>

      <div className="mt-5 border-t border-borderSubtle pt-4">
        <h3 className="text-[11.5px] font-semibold uppercase leading-4 tracking-[0.06em] text-textMuted">
          On this computer
        </h3>
        <div className="mt-3 flex flex-wrap gap-2">
          {apps.map((app) => (
            <button
              key={app.target}
              type="button"
              onClick={() => onOpen(app)}
              className={`${GLAZE_PILL} flex h-9 items-center gap-2 pl-2.5 pr-3.5 text-[12.5px]`}
            >
              <span className="flex h-[17px] w-[17px] shrink-0 items-center justify-center" aria-hidden="true">
                <SocialLogo platform={mark} size={markSize(request.service, 16)} />
              </span>
              {`Open in ${app.app}`}
            </button>
          ))}
          <button
            type="button"
            onClick={() => onOpen(null)}
            className={`${GLAZE_PILL} flex h-9 items-center gap-2 pl-2.5 pr-3.5 text-[12.5px]`}
          >
            <Globe size={15} className="shrink-0 text-textSecondary" aria-hidden="true" />
            Open in your browser
          </button>
        </div>
      </div>
    </div>
  );
}

/** What Tab can reach inside the dialog: shown, enabled controls. */
function tabStops(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>('button, a[href], [tabindex]:not([tabindex="-1"])')].filter(
    (el) => !el.hasAttribute('disabled') && el.getClientRects().length > 0,
  );
}

/** The Motion setting is Off: nothing animates. */
const motionOff = () => typeof document !== 'undefined' && document.documentElement.dataset.motion === 'off';

/** Where the card grows from, as shares of its box: the opening button's
 *  center, kept near the card so it reads as a direction, not a leap. */
function originToward(card: HTMLElement, from: HTMLElement | null): { x: number; y: number } {
  if (!from || !card.offsetWidth || !card.offsetHeight) return { x: 0.5, y: 0.5 };
  const b = from.getBoundingClientRect();
  let left = 0;
  let top = 0;
  for (let at: HTMLElement | null = card; at; at = at.offsetParent as HTMLElement | null) {
    left += at.offsetLeft;
    top += at.offsetTop;
  }
  const clamp = (v: number) => Math.min(1.25, Math.max(-0.25, v));
  return {
    x: clamp((b.left + b.width / 2 - left) / card.offsetWidth),
    y: clamp((b.top + b.height / 2 - top) / card.offsetHeight),
  };
}

export interface SongQrDialogProps {
  /** The link shown, or null while closed. */
  request: SongQrRequest | null;
  title: string;
  artist: string;
  apps: readonly SongApp[];
  /** The button that opened it: the card grows from it and the keyboard goes back to it. */
  from: HTMLElement | null;
  onClose: () => void;
  onOpen: (app: SongApp | null) => void;
}

export function SongQrDialog({ request, title, artist, apps, from, onClose, onOpen }: SongQrDialogProps) {
  const open = request !== null;
  const cardRef = useRef<HTMLDivElement>(null);
  const headingId = useId();
  const [origin, setOrigin] = useState({ x: 0.5, y: 0.5 });
  const pressedInside = useOutsidePress(open, [cardRef], onClose);
  const off = motionOff();

  // Placed before the first paint, so the pop starts from the button.
  useLayoutEffect(() => {
    if (open && cardRef.current) setOrigin(originToward(cardRef.current, from));
  }, [open, from]);

  // The keyboard moves in, and back to the button on close.
  useEffect(() => {
    if (!open) return;
    cardRef.current?.focus({ preventScroll: true });
    return () => from?.focus({ preventScroll: true });
  }, [open, from]);

  // Escape closes it, and nothing under it hears the press (full screen stays).
  useEffect(() => {
    if (!open) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [open, onClose]);

  // Tab goes round inside the card; the panel's own round stays out of it.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'Tab' || !cardRef.current) return;
    e.stopPropagation();
    const stops = tabStops(cardRef.current);
    if (stops.length === 0) {
      e.preventDefault();
      return;
    }
    const first = stops[0];
    const last = stops[stops.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === cardRef.current)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  };

  if (typeof document === 'undefined') return null;
  return createPortal(
    <AnimatePresence>
      {request && (
        <motion.div
          key="song-qr"
          {...{ [SONG_LAYER_ATTR]: '' }}
          className="fixed inset-0 flex items-center justify-center p-4"
          style={{ zIndex: 'calc(var(--sn-z-over-player-lift, 0) + 70)' }}
        >
          <motion.div
            aria-hidden="true"
            className="absolute inset-0"
            style={{ background: 'rgba(0, 0, 0, 0.55)' }}
            initial={off ? false : { opacity: 0 }}
            animate={{ opacity: 1, transition: { duration: 0.16, ease: EASE } }}
            exit={{ opacity: 0, transition: { duration: off ? 0 : 0.14, ease: EASE } }}
          />
          <motion.div
            ref={cardRef}
            role="dialog"
            aria-modal="true"
            aria-labelledby={headingId}
            tabIndex={-1}
            onPointerDownCapture={pressedInside}
            onKeyDown={onKeyDown}
            className="glass-flyout no-live-blur relative max-h-full w-[400px] max-w-full overflow-y-auto !rounded-[24px] !p-6 text-left shadow-[0_24px_60px_-12px_rgba(0,0,0,0.6)] outline-none"
            style={{ originX: origin.x, originY: origin.y }}
            initial={off ? false : { opacity: 0, scale: 0.9, y: 12 }}
            animate={{
              opacity: 1,
              scale: 1,
              y: 0,
              transition: { type: 'spring', stiffness: 420, damping: 32, opacity: { duration: 0.16, ease: EASE } },
            }}
            exit={{ opacity: 0, scale: 0.96, transition: { duration: off ? 0 : 0.14, ease: EASE } }}
          >
            <SongQrCard
              request={request}
              title={title}
              artist={artist}
              apps={apps}
              headingId={headingId}
              onClose={onClose}
              onOpen={onOpen}
            />
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
