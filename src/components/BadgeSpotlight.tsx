// Lightweight badge card shown when a badge is clicked (chat rows, profile
// cards, the chat-client badge gallery). Like EmoteSpotlight it is a quick
// look in the CURRENT window and never opens the full Badges overlay by
// itself: big art, name, provider, what Rust knows about it, and two ways out
// ("Open in Badges" for the full page, "Open website" for the provider's).
//
// Third-party badges ask Rust `get_badge_detail` (in-memory feeds) for their
// description, holder count and whether you hold them. Everything else is
// already on the clicked badge.
import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { motion } from 'framer-motion';
import { Check, ExternalLink, LayoutGrid, X } from 'lucide-react';
import { BadgeArt } from './chat/BadgeHoverCard';
import { useAppStore } from '../stores/AppStore';
import { useBadgeSpotlightStore } from '../stores/badgeSpotlightStore';
import { getBadgeDetail, type BadgeDetail } from '../services/badgeService';
import {
  badgeDetailsRoute,
  badgeProviderLabel,
  isThirdPartyBadgeProvider,
  openBadgeDetailsRoute,
  pickBadgeArt,
  safeBadgeLink,
  type BadgeRef,
} from '../utils/badgeSpotlight';
import { openExternal } from '../utils/openExternal';

/** Art edge in the card: big enough to read detail, never past twice the art. */
const SPOTLIGHT_ART_PX = 112;
/** How long the card waits for Rust before showing without its detail, so a
 *  normal (instant) answer lands before the first frame and nothing shifts. */
const DETAIL_WAIT_MS = 250;

export default function BadgeSpotlight() {
  const badge = useBadgeSpotlightStore((s) => s.badge);
  const fromBadges = useBadgeSpotlightStore((s) => s.fromBadges);
  const close = useBadgeSpotlightStore((s) => s.close);
  if (!badge) return null;
  return <SpotlightCard key={`${badge.provider}:${badge.id}:${badge.title}`} badge={badge} fromBadges={fromBadges} onClose={close} />;
}

function SpotlightCard({ badge, fromBadges, onClose }: { badge: BadgeRef; fromBadges: boolean; onClose: () => void }) {
  const thirdParty = isThirdPartyBadgeProvider(badge.provider);
  const [detail, setDetail] = useState<BadgeDetail | null>(null);
  const [settled, setSettled] = useState(!thirdParty);

  useEffect(() => {
    if (!thirdParty) return;
    let live = true;
    const timer = window.setTimeout(() => live && setSettled(true), DETAIL_WAIT_MS);
    const viewer = useAppStore.getState().currentUser?.user_id ?? null;
    getBadgeDetail(badge.provider, badge.id, badge.title, viewer)
      .then((d) => live && setDetail(d))
      .catch(() => {
        // An id Rust refuses still shows the card from the clicked badge.
      })
      .finally(() => live && setSettled(true));
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [thirdParty, badge.provider, badge.id, badge.title]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [onClose]);

  if (!settled) return null;

  const providerLabel = detail?.provider_label ?? badgeProviderLabel(badge.provider);
  const title = badge.title || detail?.title || providerLabel;
  // Twitch often repeats the title as the description, and some providers send
  // whitespace; neither is worth a line (an empty one still took its margin).
  const rawDescription = (badge.description || detail?.description || '').trim();
  const description = rawDescription && rawDescription.toLowerCase() !== title.trim().toLowerCase() ? rawDescription : null;
  // The art the viewer clicked wins; Rust's fills in when the badge had none.
  const art =
    pickBadgeArt(badge.art, SPOTLIGHT_ART_PX) ||
    pickBadgeArt(
      [detail?.image_1x, detail?.image_2x, detail?.image_4x].filter((u): u is string => !!u).map((url) => ({ url })),
      SPOTLIGHT_ART_PX,
    );
  const route = fromBadges ? null : badgeDetailsRoute(badge);
  const website = safeBadgeLink(detail?.click_url ?? badge.clickUrl ?? detail?.homepage);
  const holders = detail?.holder_count ?? null;
  const owned = detail?.owned === true;

  return createPortal(
    <div className="fixed inset-0 z-[300] flex items-center justify-center bg-black/40 backdrop-blur-2xl">
      <div className="absolute inset-0" onClick={onClose} />
      <motion.div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        initial={{ opacity: 0, scale: 0.96, y: 12 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ type: 'spring', stiffness: 360, damping: 26 }}
        className="liquid-glass-panel custom-scrollbar relative z-10 flex max-h-[86vh] w-[320px] max-w-[92vw] flex-col items-center overflow-y-auto px-6 pb-5 pt-6 text-center"
      >
        <button
          type="button"
          onClick={onClose}
          aria-label="Close"
          autoFocus
          className="absolute right-3 top-3 rounded p-1.5 text-textSecondary hover:bg-glass hover:text-textPrimary focus:outline-none focus-visible:ring-1 focus-visible:ring-accent/70"
        >
          <X size={16} />
        </button>

        <BadgeArt key={art} src={art} fallbacks={badge.fallbackUrls} box={SPOTLIGHT_ART_PX} alt={title} />

        <div className="mt-3 flex flex-col items-center gap-0.5">
          <h2 className="text-lg font-semibold leading-tight text-textPrimary [overflow-wrap:anywhere]">{title}</h2>
          {providerLabel !== title && <span className="text-xs text-textSecondary">{providerLabel}</span>}
        </div>

        {description && (
          <p className="mt-3 line-clamp-4 text-[13px] leading-snug text-textSecondary">{description}</p>
        )}

        {(holders !== null || owned) && (
          <div className="mt-3 flex items-center justify-center gap-2 text-[11px]">
            {owned && (
              <span className="flex items-center gap-1 rounded-full bg-accent/10 px-2 py-0.5 font-medium text-accent">
                <Check size={11} strokeWidth={3} />
                You have this
              </span>
            )}
            {holders !== null && (
              <span className="tabular-nums text-textMuted">
                {holders.toLocaleString()} holder{holders === 1 ? '' : 's'}
              </span>
            )}
          </div>
        )}

        {(route || website) && (
          <div className="mt-5 flex w-full flex-col items-center gap-2">
            {route && (
              <button
                type="button"
                onClick={() => {
                  onClose();
                  void openBadgeDetailsRoute(route);
                }}
                className="glass-button flex w-full items-center justify-center gap-1.5 rounded px-3 py-2 text-sm text-textPrimary"
              >
                <LayoutGrid size={14} />
                Open in Badges
              </button>
            )}
            {website && (
              <button
                type="button"
                onClick={() => void openExternal(website)}
                className="flex items-center justify-center gap-1.5 py-1 text-xs text-textSecondary hover:text-textPrimary"
              >
                <ExternalLink size={13} />
                Open website
              </button>
            )}
          </div>
        )}
      </motion.div>
    </div>,
    document.body,
  );
}
