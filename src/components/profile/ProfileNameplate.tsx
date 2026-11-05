// A member's StreamNook page as one quiet plate under their name: the mark,
// "streamnook.app/" dimmed, their name in their rank color, an arrow. Pressing
// the plate opens the page; the square at its end copies the link.
//
// One line always. When the plate runs out of room the dimmed prefix goes first
// (it wraps into a clipped second line), then the name ellipsizes. It never
// wraps and never moves anything down: a slot that is not needed collapses and a
// plate that arrives late opens its slot (height and opacity only, which
// Interface > Motion collapses to instant). The floor, edge and muted text are
// fixed colors (plateStyle.ts), so Glassiness and a light theme leave it alone.
import { useEffect, useState } from 'react';
import { ArrowUpRight, Check, Copy } from 'lucide-react';
import { ExternalAnchor } from '../ui/ExternalAnchor';
import streamnookMark from '../../assets/streamnook-logo-128.webp';
import { Logger } from '../../utils/logger';
import type { PageLink } from '../../types/profileView';
import type { PlateSlot } from './profileLink';
import {
  PLATE_CHECK,
  PLATE_COPIED_MS,
  PLATE_DIVIDER,
  PLATE_EDGE,
  PLATE_FLOOR,
  PLATE_GAP,
  PLATE_MARK,
  PLATE_MUTED,
  PLATE_NAME_WEIGHT,
  PLATE_RADIUS,
  PLATE_RESERVED_WIDTH,
  PLATE_SIZE,
  PLATE_TEXT,
  plateTopLight,
} from './plateStyle';

const PRESS =
  'transition-opacity duration-150 [@media(hover:hover)]:hover:opacity-85 active:opacity-65';

export function ProfileNameplate({
  slot,
  link,
  size = 'desktop',
}: {
  slot: PlateSlot;
  link: PageLink | null;
  size?: 'desktop' | 'phone';
}) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = window.setTimeout(() => setCopied(false), PLATE_COPIED_MS);
    return () => window.clearTimeout(t);
  }, [copied]);

  const height = PLATE_SIZE[size];
  const shown = slot === 'shown' && link !== null;
  const collapsed = slot === 'hidden';

  const copy = async () => {
    if (!link) return;
    try {
      await navigator.clipboard.writeText(link.url);
      setCopied(true);
    } catch (err) {
      Logger.error('[ProfileNameplate] could not copy the link:', err);
    }
  };

  return (
    <div
      aria-hidden={shown ? undefined : true}
      inert={!shown}
      className="overflow-hidden"
      style={{
        height: collapsed ? 0 : height + PLATE_GAP,
        opacity: collapsed ? 0 : 1,
        transition: 'height 180ms cubic-bezier(0.2, 0.8, 0.2, 1), opacity 140ms linear',
      }}
    >
      <div style={{ paddingTop: PLATE_GAP }}>
        {/* glass-exempt: a fixed floor on purpose. The plate must read the same
            at any Glassiness and on any theme, and it is not a frosted surface. */}
        <div
          className="sn-light-off relative flex w-fit max-w-full items-stretch"
          style={{
            height,
            // Reserved (or collapsing) it is the empty plate of a fixed width.
            width: shown ? undefined : PLATE_RESERVED_WIDTH,
            borderRadius: PLATE_RADIUS,
            background: PLATE_FLOOR,
            boxShadow: link
              ? `inset 0 0 0 1px ${PLATE_EDGE}, inset 0 1px 0 0 ${plateTopLight(link.accent)}`
              : `inset 0 0 0 1px ${PLATE_EDGE}`,
          }}
        >
          {shown && link && (
            <>
              <ExternalAnchor
                href={link.href}
                aria-label={`Open ${link.name}'s StreamNook page`}
                className={`flex min-w-0 flex-[0_1_auto] items-center gap-2 pl-[10px] pr-2.5 ${PRESS}`}
                style={{ borderTopLeftRadius: PLATE_RADIUS, borderBottomLeftRadius: PLATE_RADIUS }}
              >
                <img
                  src={streamnookMark}
                  alt=""
                  draggable={false}
                  width={PLATE_MARK}
                  height={PLATE_MARK}
                  className="shrink-0 object-contain"
                />
                {/* Reversed and wrapping on a one-line box: when the pair does
                    not fit, the dimmed prefix (second in the markup) is the one
                    pushed to the clipped second line. */}
                <span
                  className="flex min-w-0 flex-row-reverse flex-wrap justify-end overflow-hidden"
                  style={{ height: PLATE_MARK, lineHeight: `${PLATE_MARK}px`, fontSize: PLATE_TEXT }}
                >
                  <span
                    className="max-w-full overflow-hidden text-ellipsis whitespace-nowrap"
                    style={{ color: link.accent, fontWeight: PLATE_NAME_WEIGHT }}
                  >
                    {link.name}
                  </span>
                  <span className="whitespace-nowrap" style={{ color: PLATE_MUTED }}>
                    {link.prefix}
                  </span>
                </span>
                <ArrowUpRight size={12} className="shrink-0" style={{ color: PLATE_MUTED }} aria-hidden="true" />
              </ExternalAnchor>
              <span aria-hidden="true" className="my-[7px] w-px shrink-0" style={{ background: PLATE_DIVIDER }} />
              <button
                type="button"
                onClick={() => void copy()}
                aria-label="Copy link"
                className={`flex shrink-0 items-center justify-center ${PRESS}`}
                style={{ width: height, borderTopRightRadius: PLATE_RADIUS, borderBottomRightRadius: PLATE_RADIUS }}
              >
                {copied ? (
                  <Check size={14} style={{ color: PLATE_CHECK }} aria-hidden="true" />
                ) : (
                  <Copy size={14} style={{ color: PLATE_MUTED }} aria-hidden="true" />
                )}
              </button>
              <span className="sr-only" aria-live="polite">
                {copied ? 'Link copied' : ''}
              </span>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
