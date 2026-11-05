// Whether the sheet's decorative motion rests: Battery Saver, or Interface >
// Motion set to Off. (The CSS loops rest on their own under both; this is for
// the art CSS cannot pause, like an animated image.)
import { useAppStore } from '../../stores/AppStore';
import { useLowPower } from '../../utils/lowPower';

export function useHoldStill(): boolean {
  const lowPower = useLowPower();
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
  return lowPower || motionOff;
}
