import { useCallback, useEffect, useRef, useState } from 'react';

// State behind the player's volume readout (see PlayerVolumeOsd). Lives here
// rather than beside the component so the component file exports only a
// component, which is what keeps fast refresh working.

const VISIBLE_MS = 1000;

export interface VolumeOsdState {
  volume: number;
  muted: boolean;
}

/** Shows the readout on demand and hides it again after a beat. */
export function useVolumeOsd() {
  const [osd, setOsd] = useState<VolumeOsdState | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Each call replaces the value and restarts the hide timer, so a run of
  // scrolling keeps one pill alive and updating rather than showing a new one
  // per notch.
  const showOsd = useCallback((volume: number, muted: boolean) => {
    setOsd({ volume, muted });
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      setOsd(null);
    }, VISIBLE_MS);
  }, []);

  useEffect(() => () => {
    if (timerRef.current) clearTimeout(timerRef.current);
  }, []);

  return { osd, showOsd };
}
