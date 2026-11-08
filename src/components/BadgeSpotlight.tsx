// Lightweight badge card shown when a badge is clicked (chat rows, profile
// cards, the chat-client badge gallery). Like EmoteSpotlight it is a quick
// look in the CURRENT window and never opens the full Badges overlay by
// itself: big art, name, provider, what Rust knows about it, and two ways out
// ("Open in Badges" for the full page, "Open website" for the provider's).
//
// Third-party badges ask Rust `get_badge_detail` (in-memory feeds) for their
// description, holder count and whether you hold them (`hooks/useBadgeCard`,
// shared with the phone's badge sheet). Everything else is already on the
// clicked badge.
import { useEffect } from 'react';
import { createPortal } from 'react-dom';
import { motion } from 'framer-motion';
import { Check, ExternalLink, LayoutGrid, X } from 'lucide-react';
import { BadgeArt } from './chat/BadgeHoverCard';
import { useBadgeSpotlightStore } from '../stores/badgeSpotlightStore';
import { badgeCardFacts, useBadgeDetail } from '../hooks/useBadgeCard';
import { badgeDetailsRoute, openBadgeDetailsRoute, type BadgeRef } from '../utils/badgeSpotlight';
import { openExternal } from '../utils/openExternal';

/** Art edge in the card: big enough to read detail, never past twice the art. */
const SPOTLIGHT_ART_PX = 112;

export default function BadgeSpotlight() {
  const badge = useBadgeSpotlightStore((s) => s.badge);
  const fromBadges = useBadgeSpotlightStore((s) => s.fromBadges);
  const close = useBadgeSpotlightStore((s) => s.close);
  if (!badge) return null;
  return <SpotlightCard key={`${badge.provider}:${badge.id}:${badge.title}`} badge={badge} fromBadges={fromBadges} onClose={close} />;
}

function SpotlightCard({ badge, fromBadges, onClose }: { badge: BadgeRef; fromBadges: boolean; onClose: () => void }) {
  const { detail, settled } = useBadgeDetail(badge);

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

  const { title, providerLabel, description, art, website, holders, owned } = badgeCardFacts(badge, detail, SPOTLIGHT_ART_PX);
  const route = fromBadges ? null : badgeDetailsRoute(badge);

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
