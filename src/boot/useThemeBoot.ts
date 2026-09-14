// Shared theme boot: resolves and applies the palette, glass strength, and
// interface font from settings. Called by BOTH shells (desktop App.tsx and
// mobile MobileApp.tsx) so a theme change behaves identically everywhere.
// Extracted verbatim from App.tsx's theme effect; keep the two shells' boot
// sequences in sync (see src/mobile/boot/useMobileBoot.ts for the mobile list).
import { useEffect } from 'react';
import { useAppStore } from '../stores/AppStore';
import {
  applyFont,
  applyGlassStrength,
  applyTheme,
  DEFAULT_FONT_ID,
  DEFAULT_GLASS_TRANSPARENCY,
  DEFAULT_THEME_ID,
  getOledTheme,
  getThemeById,
  getThemeByIdWithCustom,
  OLED_THEME_ID,
} from '../themes';
import { Logger } from '../utils/logger';

export function useThemeBoot(): void {
  const settings = useAppStore(state => state.settings);

  // Apply theme when settings are loaded or theme changes
  useEffect(() => {
    const themeId = settings.theme || DEFAULT_THEME_ID;
    const customThemes = settings.custom_themes || [];
    // OLED is the one configurable signature theme: its accent comes from the
    // saved oled_accent, so resolve it through getOledTheme rather than the
    // static registry entry.
    const theme = themeId === OLED_THEME_ID
      ? getOledTheme(settings.oled_accent)
      : (getThemeByIdWithCustom(themeId, customThemes) || getThemeById(DEFAULT_THEME_ID));
    if (theme) {
      Logger.debug('[App] Applying theme:', theme.name);
      applyTheme(theme);
    }
    // Global glassiness is independent of the palette, so re-assert it whenever
    // the theme is (re)applied as well as when the slider itself changes.
    applyGlassStrength(settings.glass_transparency ?? DEFAULT_GLASS_TRANSPARENCY);
    // Interface font is also palette-independent; re-assert alongside the theme.
    applyFont(settings.font ?? DEFAULT_FONT_ID, settings.font_custom);
  }, [settings.theme, settings.custom_themes, settings.glass_transparency, settings.font, settings.font_custom, settings.oled_accent]);
}
