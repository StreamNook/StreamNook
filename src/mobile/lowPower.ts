// Follows the system Battery Saver. MainActivity reports flips (sn:low-power)
// and answers isPowerSaveMode() synchronously for boot and for every return
// to the foreground. The core is told too (set_low_power).
// No setting of our own: Battery Saver IS the switch.
import { invoke } from '@tauri-apps/api/core';
import { isPowerSaveMode } from './nativeBridge';
import { setLowPower } from '../utils/lowPower';
import { Logger } from '../utils/logger';

// Read once at module load, before the shell's first render, so a phone
// already on Battery Saver never draws a frame of animated emotes first.
// installLowPower keeps it current and tells the core.
if (typeof window !== 'undefined') setLowPower(isPowerSaveMode() ?? false);

function tellCore(on: boolean): void {
  void invoke('set_low_power', { on }).catch((err) => Logger.debug('[LowPower] set_low_power failed:', err));
}

function apply(on: boolean): void {
  if (!setLowPower(on)) return;
  Logger.info(`[LowPower] Battery Saver ${on ? 'on' : 'off'}`);
  tellCore(on);
}

export function installLowPower(): () => void {
  const initial = isPowerSaveMode() ?? false;
  setLowPower(initial);
  tellCore(initial);
  const onEvent = (e: Event) => apply(!!(e as CustomEvent<boolean>).detail);
  const onVisible = () => {
    if (document.visibilityState !== 'visible') return;
    const now = isPowerSaveMode();
    if (now !== null) apply(now);
  };
  window.addEventListener('sn:low-power', onEvent);
  document.addEventListener('visibilitychange', onVisible);
  return () => {
    window.removeEventListener('sn:low-power', onEvent);
    document.removeEventListener('visibilitychange', onVisible);
  };
}
