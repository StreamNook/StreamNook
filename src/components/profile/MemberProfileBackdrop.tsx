// The member's profile backdrop over a whole panel: their StreamNook atmosphere
// (the subscriber tier), else the subtle breathing tier aura.

import { motion } from 'framer-motion';
import { AtmosphereBackground } from '../AtmosphereBackground';
import { useAppStore } from '../../stores/AppStore';
import { useLowPower } from '../../utils/lowPower';
import type { MemberProfileView } from './memberProfile';

export function MemberProfileBackdrop({ view }: { view: MemberProfileView }) {
  const { hasAtmosphere, effectiveTheme, tierAura } = view;
  // The cheap look and Motion Off hold the aura still. A running framer loop
  // stops only when both `animate` and `transition` change.
  const lowPower = useLowPower();
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
  const holdStill = lowPower || motionOff;
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
          animate={holdStill ? { opacity: 0.85 } : { opacity: [0.7, 1, 0.7] }}
          transition={holdStill ? { duration: 0 } : { duration: 7, repeat: Infinity, ease: 'easeInOut' }}
        />
      ) : null}
    </>
  );
}
