// The member's profile backdrop over a whole panel: their StreamNook atmosphere
// (the subscriber tier), else the subtle breathing tier aura.

import { motion } from 'framer-motion';
import { AtmosphereBackground } from '../AtmosphereBackground';
import type { MemberProfileView } from './memberProfile';

export function MemberProfileBackdrop({ view }: { view: MemberProfileView }) {
  const { hasAtmosphere, effectiveTheme, tierAura } = view;
  return (
    <>
      {/* Whole-overlay backdrop. Only the Atmosphere (subscriber tier) fills
          the ENTIRE profile (it IS the vibe), softened to an ambient wash
          behind the content. A 7TV paint theme does NOT go here — it's a
          hero-only accent (below), so it stays clearly below the atmospheres.
          Everything else gets the subtle tier radial (incl. behind a paint
          theme). */}
      {hasAtmosphere ? (
        <AtmosphereBackground
          atm={effectiveTheme!.atmosphere!}
          variant="profile"
          blur={!!effectiveTheme!.atmosphere!.image}
        />
      ) : tierAura ? (
        <motion.div
          className="pointer-events-none absolute -inset-10"
          style={{ background: tierAura }}
          animate={{ opacity: [0.7, 1, 0.7] }}
          transition={{ duration: 7, repeat: Infinity, ease: 'easeInOut' }}
        />
      ) : null}
    </>
  );
}
