import { useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Sparkles } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import {
  subscribeCosmeticsVersion,
  getCosmeticsVersion,
  getUserCosmeticSlugs,
  getCosmeticBySlug,
} from '../services/supabaseService';
import { COSMETIC_ASSET_CHAT_BY_SLUG } from './cosmeticAssets';

// Paid tiers worth celebrating, highest first (a subscriber payment also grants
// the supporter badge, so prefer announcing the higher tier). The free default
// "Member" badge is intentionally absent — everyone has it; it is not an unlock.
const PAID_SLUGS = ['streamnook-subscriber', 'streamnook-supporter'] as const;

const NOTE_DURATION_MS = 7000;

/**
 * A small, transient glass note that confirms a tier unlock the instant a paid
 * entitlement lands for the CURRENT user — e.g. right after they complete a
 * purchase on streamnook.app, delivered live through the cosmetics realtime
 * channel (no app restart). Deliberately a quiet auto-fading note rather than a
 * toast: this fires at most a couple of times in a user's lifetime, so it should
 * feel like a small reward, not a routine notification.
 *
 * Only a GROWTH in entitlements triggers it. The first observation after the
 * registry loads is recorded as a silent baseline, so existing supporters never
 * see it on launch.
 */
export const EntitlementUnlockNote = () => {
  const userId = useAppStore((s) => s.currentUser?.user_id);

  // The entitlement slugs we've already accounted for. Null until a baseline is
  // taken (which only happens once the registry has actually loaded), so the
  // initial load is never mistaken for a new unlock.
  const knownSlugsRef = useRef<Set<string> | null>(null);
  const baselineUserIdRef = useRef<string | undefined>(undefined);
  const [unlocked, setUnlocked] = useState<{ slug: string; name: string } | null>(null);

  useEffect(() => {
    if (!userId) {
      knownSlugsRef.current = null;
      baselineUserIdRef.current = undefined;
      return;
    }

    // Record what the user already owns so only a FUTURE growth counts as an
    // unlock. Never notifies. No-op until the registry has loaded (version 0),
    // so we don't baseline against an empty set and then "discover" their
    // existing badges as brand new.
    const establishBaseline = (): boolean => {
      if (getCosmeticsVersion() === 0) return false;
      baselineUserIdRef.current = userId;
      // getUserCosmeticSlugs returns ONLY explicit entitlements (it excludes the
      // is_default Member badge), which is exactly the paid-grant set we track.
      knownSlugsRef.current = new Set(getUserCosmeticSlugs(userId));
      return true;
    };

    // Fires on every cosmetics-registry change (load complete, realtime grant,
    // focus refetch). Calling setState here is the endorsed pattern: state set
    // from an external-system subscription callback, not synchronously in the
    // effect body.
    const onChange = () => {
      if (knownSlugsRef.current === null || baselineUserIdRef.current !== userId) {
        establishBaseline();
        return;
      }
      const current = getUserCosmeticSlugs(userId);
      const known = knownSlugsRef.current;
      let fresh: string | null = null;
      for (const slug of PAID_SLUGS) {
        if (current.has(slug) && !known.has(slug)) { fresh = slug; break; }
      }
      // Fold everything currently owned into the baseline so a single grant only
      // ever fires once, even across multiple registry reloads.
      knownSlugsRef.current = new Set(current);
      if (fresh) {
        const name = getCosmeticBySlug(fresh)?.name ?? 'StreamNook';
        setUnlocked({ slug: fresh, name });
      }
    };

    // Baseline now if the registry is already loaded; otherwise onChange will
    // take the baseline on the first load it observes.
    establishBaseline();
    const unsub = subscribeCosmeticsVersion(onChange);
    return unsub;
  }, [userId]);

  useEffect(() => {
    if (!unlocked) return;
    const t = setTimeout(() => setUnlocked(null), NOTE_DURATION_MS);
    return () => clearTimeout(t);
  }, [unlocked]);

  const asset = unlocked ? COSMETIC_ASSET_CHAT_BY_SLUG[unlocked.slug] : null;

  return (
    <AnimatePresence>
      {unlocked && (
        <motion.div
          initial={{ opacity: 0, x: -8 }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: -8 }}
          transition={{ duration: 0.3, ease: 'easeInOut' }}
          className="fixed bottom-4 left-4 z-[60] max-w-xs pointer-events-none"
        >
          <div className="glass-panel p-3 rounded-lg border border-accent/30 bg-background/80 backdrop-blur-md">
            <div className="flex items-start gap-2.5">
              {asset && (
                <img
                  src={asset}
                  alt=""
                  className="w-7 h-7 object-contain flex-shrink-0 mt-0.5"
                  draggable={false}
                />
              )}
              <div className="space-y-1">
                <p className="text-textPrimary text-xs font-semibold flex items-center gap-1.5">
                  <Sparkles className="w-3.5 h-3.5 text-amber-300 flex-shrink-0" />
                  {unlocked.name} unlocked
                </p>
                <p className="text-textSecondary text-xs leading-relaxed">
                  Your badge and perks are active now. Thanks for supporting StreamNook.
                </p>
              </div>
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};

export default EntitlementUnlockNote;
