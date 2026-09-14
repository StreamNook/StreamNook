// System-bar icon colour, driven by the active StreamNook theme.
//
// Android draws the app behind the status and navigation bars, so what sits
// under the clock and the battery icon is the theme's background and nothing
// else. Left to itself, `enableEdgeToEdge()` picks the icon colour from
// `SystemBarStyle.auto`, whose detector reads the PHONE's night-mode setting -
// a preference that says nothing about which StreamNook theme is loaded. A
// phone in light mode therefore drew a black clock over StreamNook's dark
// background, and a light theme on a phone in dark mode would draw a white one
// over a near-white bar.
//
// So the app states what it needs instead of letting the OS guess. `applyTheme`
// announces every palette change; this converts that into the one native call
// and holds it there.
import { setBarsLightContent } from './nativeBridge';
import { needsLightContentOn } from '../themes';
import { Logger } from '../utils/logger';

interface ThemeAppliedDetail {
  background: string;
  lightContent: boolean;
}

let installed = false;

/**
 * Idempotent; returns a teardown.
 *
 * Installing does NOT assume it beat the first `applyTheme`: main.tsx reaches
 * this through a dynamic import, which races the boot effect. So it reads
 * whatever palette is already on the document first and only then listens. If
 * nothing has been applied yet the variable is empty and the event covers it.
 */
export function installSystemBarAppearance(): () => void {
  if (installed) return () => {};
  installed = true;

  const apply = (light: boolean, source: string) => {
    Logger.debug('[SystemBars] light content:', light, `(${source})`);
    setBarsLightContent(light);
  };

  const current = getComputedStyle(document.documentElement)
    .getPropertyValue('--color-background')
    .trim();
  if (current) apply(needsLightContentOn(current), current);

  const onTheme = (event: Event) => {
    const detail = (event as CustomEvent<ThemeAppliedDetail>).detail;
    if (typeof detail?.lightContent !== 'boolean') return;
    apply(detail.lightContent, detail.background);
  };

  window.addEventListener('sn:theme-applied', onTheme);
  return () => {
    window.removeEventListener('sn:theme-applied', onTheme);
    installed = false;
  };
}
