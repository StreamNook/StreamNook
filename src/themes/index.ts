// StreamNook Theme System
// All themes follow a consistent structure for easy switching

export interface ThemePalette {
    // Core colors
    background: string;
    backgroundSecondary: string;
    backgroundTertiary: string;

    // Surface colors (for glass panels, cards, etc.)
    surface: string;
    surfaceHover: string;
    surfaceActive: string;

    // Text colors
    textPrimary: string;
    textSecondary: string;
    textMuted: string;

    // Accent colors
    accent: string;
    accentHover: string;
    accentMuted: string;
    /** The accent floored to 3:1 on the raised surface (Rust's
     *  theme_palette::accent_on_surface), for accent marks that sit on a card:
     *  share bars, accent words and icons. Every built-in ships it; a palette
     *  without it paints the accent. */
    accentOnSurface?: string;

    // Border colors
    border: string;
    borderLight: string;
    borderSubtle: string;

    // Semantic colors
    success: string;
    warning: string;
    error: string;
    info: string;

    // Special colors
    scrollbarThumb: string;
    scrollbarTrack: string;

    // Glass effect opacities
    glassOpacity: string;
    glassHoverOpacity: string;
    glassActiveOpacity: string;

    // Syntax/highlight colors (for chat, code, etc.)
    highlight: {
        pink: string;
        purple: string;
        blue: string;
        cyan: string;
        green: string;
        yellow: string;
        orange: string;
        red: string;
    };
}

export interface Theme {
    id: string;
    name: string;
    description: string;
    category: 'signature' | 'universal' | 'modern' | 'classic' | 'cozy';
    palette: ThemePalette;
    /** A light treatment that goes beyond the palette. applyTheme writes it to
     *  `data-theme-effect` on <html>, and styles/theme-prism.css keys off it.
     *  Palette-only themes leave it unset. */
    effect?: 'prism';
}

// ============================================
// BUILT-IN THEMES
// ============================================

// The built-in palettes live in builtinThemes.json, which the Rust core reads
// too (services/theme_palette.rs), so every shell paints from one copy. The
// order there is the gallery order.
import builtIn from './builtinThemes.json';

export const themes: Theme[] = builtIn.themes as Theme[];

export const themeCategories: { id: Theme['category']; name: string; description: string }[] =
    builtIn.categories as { id: Theme['category']; name: string; description: string }[];

// The OLED theme is the one configurable signature theme: pick any accent and
// the rest of its accent-derived colors follow. These exports drive that picker.
export const OLED_THEME_ID = 'prince0fdubai-oled';

// Default accent = the original OLED purple, so existing users see no change.
export const DEFAULT_OLED_ACCENT = '#a064ff';

// One-click accents shown when OLED is selected. Purple is the original; orange
// preserves the look of the retired second OLED variant; the rest cover common
// tastes. Users can also dial in any color with the full picker.
export const OLED_ACCENT_PRESETS: { name: string; value: string }[] = [
    { name: 'Purple', value: '#a064ff' },
    { name: 'Orange', value: '#ff9933' },
    { name: 'Blue', value: '#66b3ff' },
    { name: 'Cyan', value: '#22d3ee' },
    { name: 'Green', value: '#22c55e' },
    { name: 'Pink', value: '#ff66b2' },
    { name: 'Red', value: '#ff4466' },
    { name: 'Gold', value: '#ffcc00' },
];

const oledTheme: Theme = themes.find((t) => t.id === OLED_THEME_ID)!;

// How far OLED's secondary text mixes its accent toward white. It lives in the
// shared JSON so the Rust core derives the same color, and the core's tests pin
// it: even a black accent reaches the 4.5:1 secondary floor at this mix.
export const OLED_SECONDARY_LIGHTEN: number = builtIn.oledSecondaryLighten;

// Helper functions
export const getThemeById = (id: string): Theme | undefined => {
    return themes.find((theme) => theme.id === id);
};

export const getThemesByCategory = (category: Theme['category']): Theme[] => {
    return themes.filter((theme) => theme.category === category);
};

// ============================================
// CUSTOM THEME UTILITIES
// ============================================

import { invoke } from '@tauri-apps/api/core';
import type { CustomTheme, CustomThemeColor, CustomThemePalette } from '../types';
import { IS_MOBILE } from '../utils/platform';

// Resolve a CustomThemeColor to a CSS-compatible string
const resolveColor = (c: CustomThemeColor): string => {
    if (c.opacity < 100) {
        // Convert hex to rgba with opacity
        const hex = c.value.replace('#', '');
        const r = parseInt(hex.slice(0, 2), 16);
        const g = parseInt(hex.slice(2, 4), 16);
        const b = parseInt(hex.slice(4, 6), 16);
        return `rgba(${r}, ${g}, ${b}, ${c.opacity / 100})`;
    }
    return c.value;
};

// Convert CustomTheme to runtime Theme format
export const customThemeToTheme = (custom: CustomTheme): Theme => {
    const p = custom.palette;
    return {
        id: custom.id,
        name: custom.name,
        description: `Custom theme created ${new Date(custom.createdAt).toLocaleDateString()}`,
        category: 'signature', // Custom themes appear in signature category
        palette: {
            background: resolveColor(p.background),
            backgroundSecondary: resolveColor(p.backgroundSecondary),
            backgroundTertiary: resolveColor(p.backgroundTertiary),
            surface: resolveColor(p.surface),
            surfaceHover: resolveColor(p.surfaceHover),
            surfaceActive: resolveColor(p.surfaceActive),
            textPrimary: resolveColor(p.textPrimary),
            textSecondary: resolveColor(p.textSecondary),
            textMuted: resolveColor(p.textMuted),
            accent: resolveColor(p.accent),
            accentHover: resolveColor(p.accentHover),
            accentMuted: resolveColor(p.accentMuted),
            // Rust writes the floored accent into custom themes on load; one
            // saved before that has none and paints its accent.
            accentOnSurface: resolveColor(p.accentOnSurface ?? p.accent),
            border: resolveColor(p.border),
            borderLight: resolveColor(p.borderLight),
            borderSubtle: resolveColor(p.borderSubtle),
            success: resolveColor(p.success),
            warning: resolveColor(p.warning),
            error: resolveColor(p.error),
            info: resolveColor(p.info),
            scrollbarThumb: resolveColor(p.scrollbarThumb),
            scrollbarTrack: resolveColor(p.scrollbarTrack),
            glassOpacity: p.glassOpacity,
            glassHoverOpacity: p.glassHoverOpacity,
            glassActiveOpacity: p.glassActiveOpacity,
            highlight: {
                pink: resolveColor(p.highlight.pink),
                purple: resolveColor(p.highlight.purple),
                blue: resolveColor(p.highlight.blue),
                cyan: resolveColor(p.highlight.cyan),
                green: resolveColor(p.highlight.green),
                yellow: resolveColor(p.highlight.yellow),
                orange: resolveColor(p.highlight.orange),
                red: resolveColor(p.highlight.red),
            },
        },
    };
};

// Parse color to extract hex value (handles rgba, rgb, and hex formats)
const parseColorToHex = (color: string): string => {
    if (color.startsWith('#')) {
        return color.length === 4
            ? `#${color[1]}${color[1]}${color[2]}${color[2]}${color[3]}${color[3]}`
            : color;
    }
    if (color.startsWith('rgba') || color.startsWith('rgb')) {
        const match = color.match(/rgba?\((\d+),\s*(\d+),\s*(\d+)/);
        if (match) {
            const r = parseInt(match[1]).toString(16).padStart(2, '0');
            const g = parseInt(match[2]).toString(16).padStart(2, '0');
            const b = parseInt(match[3]).toString(16).padStart(2, '0');
            return `#${r}${g}${b}`;
        }
    }
    return '#000000';
};

// Parse color to extract opacity (0-100)
const parseColorOpacity = (color: string): number => {
    if (color.startsWith('rgba')) {
        const match = color.match(/rgba\([^,]+,[^,]+,[^,]+,\s*([0-9.]+)\)/);
        if (match) {
            return Math.round(parseFloat(match[1]) * 100);
        }
    }
    return 100;
};

// Create default custom theme palette based on an existing theme
export const createDefaultCustomPalette = (baseTheme: Theme): CustomThemePalette => {
    const makeColor = (value: string): CustomThemeColor => ({
        value: parseColorToHex(value),
        opacity: parseColorOpacity(value),
    });
    const p = baseTheme.palette;

    return {
        background: makeColor(p.background),
        backgroundSecondary: makeColor(p.backgroundSecondary),
        backgroundTertiary: makeColor(p.backgroundTertiary),
        surface: makeColor(p.surface),
        surfaceHover: makeColor(p.surfaceHover),
        surfaceActive: makeColor(p.surfaceActive),
        textPrimary: makeColor(p.textPrimary),
        textSecondary: makeColor(p.textSecondary),
        textMuted: makeColor(p.textMuted),
        accent: makeColor(p.accent),
        accentHover: makeColor(p.accentHover),
        accentMuted: makeColor(p.accentMuted),
        border: makeColor(p.border),
        borderLight: makeColor(p.borderLight),
        borderSubtle: makeColor(p.borderSubtle),
        success: makeColor(p.success),
        warning: makeColor(p.warning),
        error: makeColor(p.error),
        info: makeColor(p.info),
        scrollbarThumb: makeColor(p.scrollbarThumb),
        scrollbarTrack: makeColor(p.scrollbarTrack),
        glassOpacity: p.glassOpacity,
        glassHoverOpacity: p.glassHoverOpacity,
        glassActiveOpacity: p.glassActiveOpacity,
        highlight: {
            pink: makeColor(p.highlight.pink),
            purple: makeColor(p.highlight.purple),
            blue: makeColor(p.highlight.blue),
            cyan: makeColor(p.highlight.cyan),
            green: makeColor(p.highlight.green),
            yellow: makeColor(p.highlight.yellow),
            orange: makeColor(p.highlight.orange),
            red: makeColor(p.highlight.red),
        },
    };
};

// Get theme by ID, checking custom themes first
export const getThemeByIdWithCustom = (id: string, customThemes?: CustomTheme[]): Theme | undefined => {
    // Check custom themes first
    const custom = customThemes?.find((t) => t.id === id);
    if (custom) return customThemeToTheme(custom);
    
    // Fall back to built-in themes
    return themes.find((theme) => theme.id === id);
};

export const DEFAULT_THEME_ID = 'winters-glass';

// Helper function to lighten a hex color for neon effect
const lightenColor = (hex: string, percent: number): string => {
    // Handle rgb/rgba colors
    if (hex.startsWith('rgb')) {
        // For rgba colors, just return the base accent hover which is usually lighter
        return hex;
    }

    // Remove # if present
    hex = hex.replace('#', '');

    // Parse hex values
    let r = parseInt(hex.substring(0, 2), 16);
    let g = parseInt(hex.substring(2, 4), 16);
    let b = parseInt(hex.substring(4, 6), 16);

    // Lighten by mixing with white
    r = Math.min(255, Math.round(r + (255 - r) * (percent / 100)));
    g = Math.min(255, Math.round(g + (255 - g) * (percent / 100)));
    b = Math.min(255, Math.round(b + (255 - b) * (percent / 100)));

    // Convert back to hex
    return `#${r.toString(16).padStart(2, '0')}${g.toString(16).padStart(2, '0')}${b.toString(16).padStart(2, '0')}`;
};

// Split a #rrggbb string into its channels (for building rgba() strings).
const hexToRgb = (hex: string): { r: number; g: number; b: number } => {
    const h = hex.replace('#', '');
    return {
        r: parseInt(h.slice(0, 2), 16),
        g: parseInt(h.slice(2, 4), 16),
        b: parseInt(h.slice(4, 6), 16),
    };
};

/**
 * A phone's page floors: nothing shows behind a phone's app (desktop's window
 * can be see-through, a phone's never is), so a custom theme's see-through
 * background is what it looks like over black, and the tertiary floor sits on
 * that. Flattened once here, every surface built as solid from them (static
 * popovers, menus, the tab bar floor, fades, Glassiness 0) is solid.
 */
export const phoneFloors = (background: string, tertiary: string): { background: string; tertiary: string } =>
    opaqueFloors(background, tertiary, PHONE_FLOOR);

// Whether a background needs light-coloured content drawn on top of it.
// Relative luminance with the sRGB coefficients, split at the midpoint. Used for
// the Android system bars, which sit over the app's own background: only the
// palette knows what is behind the clock and the battery icon.
/** Whether the applied theme's background is dark, i.e. text on it needs to
 *  be light. Set by applyTheme; read by the readable-name-color adjustment. */
let themeDark = true;
export const isThemeDark = (): boolean => themeDark;

export const needsLightContentOn = (color: string): boolean => {
    const { r, g, b } = hexToRgb(parseColorToHex(color));
    return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255 < 0.5;
};

// What a theme color looks like painted over an opaque base, returned opaque
// (#rrggbb): the source-over the compositor would do.
const overOpaque = (color: string, base: { r: number; g: number; b: number }): string => {
    const a = color.trim() === 'transparent' ? 0 : parseColorOpacity(color) / 100;
    const { r, g, b } = hexToRgb(parseColorToHex(color));
    const mix = (c: number, under: number) => Math.round(c * a + under * (1 - a)).toString(16).padStart(2, '0');
    return `#${mix(r, base.r)}${mix(g, base.g)}${mix(b, base.b)}`;
};

// What sits under the page. Every desktop window paints this before the page
// (tauri.conf.json backgroundColor, the recreated main window and the popouts
// in Rust); a phone shows nothing behind the app.
export const WINDOW_FLOOR = '#0c0c0d';
export const PHONE_FLOOR = '#000000';

/**
 * The page floors as solid colors. A custom theme color can carry alpha, and
 * every surface recipe (popovers, menus, modals, toasts, the dock, the
 * Glassiness 0 bases, bg-background) mixes the background or tertiary color as
 * if it were opaque, so a see-through floor leaves all of them see-through.
 * The background is flattened over what is under the page and tertiary over
 * that; the page itself looks the same, only the surfaces stop reading through.
 */
export const opaqueFloors = (
    background: string,
    tertiary: string,
    under: string,
): { background: string; tertiary: string } => {
    const page = overOpaque(background, hexToRgb(parseColorToHex(under)));
    return { background: page, tertiary: overOpaque(tertiary, hexToRgb(page)) };
};

// Resolve the OLED theme for a chosen accent. OLED is the one signature theme
// whose accent the user picks; this fills the accent-derived slots (hover/muted,
// secondary text, scrollbar) from a single hex so the look stays cohesive on
// pure black. Used both to paint the live theme (via applyTheme) and to preview
// the OLED card with the chosen color. An invalid/empty hex falls back to the
// default accent, so a bad saved value can never break the paint.
export const getOledTheme = (accentHex?: string): Theme => {
    const accent = accentHex && /^#[0-9a-fA-F]{6}$/.test(accentHex) ? accentHex : DEFAULT_OLED_ACCENT;
    const { r, g, b } = hexToRgb(accent);
    return {
        ...oledTheme,
        palette: {
            ...oledTheme.palette,
            accent,
            accentHover: lightenColor(accent, 20),
            accentMuted: `rgba(${r}, ${g}, ${b}, 0.5)`,
            // Every preset already clears 3:1 on OLED's near-black cards (the
            // Rust core's tests prove it), so the accent paints first; a picked
            // accent that does not is floored by applyTheme's ask to Rust.
            accentOnSurface: accent,
            // Soft tint of the accent (lavender for purple, peach for orange…),
            // mixed toward white so secondary text stays readable on black.
            textSecondary: lightenColor(accent, OLED_SECONDARY_LIGHTEN),
            scrollbarThumb: `rgba(${r}, ${g}, ${b}, 0.25)`,
        },
    };
};

// ─── Global glassiness ───────────────────────────────────────────────────────
// The Glassiness slider (Theme settings) scales every glass surface from the
// signature frosted, see-through look (100%) down to a completely flat, solid,
// blur-free interface (0%) for users who don't want any glass at all. Two things
// move together for that to read as "no glass": the surface TINTS lose their
// transparency, and the backdrop BLUR goes.
//
// Transparency is handled here in JS by recomputing --color-surface* live, so
// every consumer degrades in lockstep — the .glass-* classes, Tailwind
// `bg-surface`, and inline var(--color-surface) reads alike — with no per-surface
// CSS. Blur is handled in globals.css: the main glass classes scale their blur by
// --glass-strength, and html[data-glass="off"] hard-strips any remaining
// backdrop-filter at 0%.

// The untouched per-theme surface tints, captured on every applyTheme so the
// slider can re-derive the live colours without re-running the whole theme.
let activeSurfaceTints: {
    surface: string;
    surfaceHover: string;
    surfaceActive: string;
    tertiary: string;
} | null = null;

// Last glassiness applied (0..1), so applyTheme can repaint surfaces at the
// user's chosen level instead of flashing full glass on a theme switch.
let lastGlassStrength = 1;

// Blur behind glass (Theme > Glassiness). Without the frost, a panel at full
// glassiness would show the content behind it sharp enough to fight the text on
// it, so with blur off the tint never gets more see-through than this. The
// cap matches the member card's pinned strength, the app's existing "glass
// that still reads" level. It lives in the shared JSON so the iPhone's native
// theme light caps glassiness the same way.
let glassBlurEnabled = true;
const BLUR_OFF_MAX_STRENGTH: number = builtIn.glassBlurOffMaxStrength;
const effectiveGlassStrength = (): number =>
    glassBlurEnabled ? lastGlassStrength : Math.min(lastGlassStrength, BLUR_OFF_MAX_STRENGTH);

// Blend a translucent surface tint toward a solid colour as glassiness drops.
// At strength 1 the original tint is returned (signature glass); at strength 0
// the tint is composited over the theme's opaque tertiary surface (source-over)
// and returned fully opaque; in between, colour and alpha interpolate, so the
// slider is a smooth continuum rather than an on/off switch.
const blendSurfaceForGlass = (tint: string, tertiaryHex: string, strength: number): string => {
    const a = parseColorOpacity(tint) / 100;
    const { r, g, b } = hexToRgb(parseColorToHex(tint));
    const base = hexToRgb(parseColorToHex(tertiaryHex));
    // Opaque source-over of the tint onto the tertiary surface.
    const solidR = r * a + base.r * (1 - a);
    const solidG = g * a + base.g * (1 - a);
    const solidB = b * a + base.b * (1 - a);
    // Interpolate raw (strength 1) → solid (strength 0).
    const lr = Math.round(r * strength + solidR * (1 - strength));
    const lg = Math.round(g * strength + solidG * (1 - strength));
    const lb = Math.round(b * strength + solidB * (1 - strength));
    const la = a * strength + (1 - strength);
    return `rgba(${lr}, ${lg}, ${lb}, ${la.toFixed(3)})`;
};

// Paint the live --color-surface* vars for a given glassiness (0..1).
const writeSurfaceGlass = (strength: number): void => {
    if (!activeSurfaceTints) return;
    const root = document.documentElement;
    const { surface, surfaceHover, surfaceActive, tertiary } = activeSurfaceTints;
    root.style.setProperty('--color-surface', blendSurfaceForGlass(surface, tertiary, strength));
    root.style.setProperty('--color-surface-hover', blendSurfaceForGlass(surfaceHover, tertiary, strength));
    root.style.setProperty('--color-surface-active', blendSurfaceForGlass(surfaceActive, tertiary, strength));
};

// ─── Rust's contrast floors ──────────────────────────────────────────────────
// The core (`lift_theme_palette`, services/theme_palette.rs) lifts muted and
// secondary text to their contrast floors and floors the accent for cards.
// Built-ins ship those values, but a custom theme, OLED in a picked accent and a
// theme being edited only get them from Rust, so every paint asks. The palette
// paints first with what it holds; the answer lands a moment later.

/** The palette keys Rust floors, and the variable each one paints. */
const FLOORED_VARS = [
    ['accentOnSurface', '--color-accent-on-surface'],
    ['textMuted', '--color-text-muted'],
    ['textSecondary', '--color-text-secondary'],
] as const;

type FlooredKey = (typeof FLOORED_VARS)[number][0];

// Every paint bumps this, so only the newest request's answer paints: dragging
// an accent in the editor fires many, and one answering late must not land over
// a newer color.
let floorSeq = 0;
// The theme whose floored values are on screen from Rust. Repainting that same
// theme (the glass slider, a re-sent settings object, a drag in the editor)
// keeps them until the new answer, instead of flashing the unfloored colors
// for a frame on every repaint.
let flooredThemeId: string | null = null;

const paintFloors = (root: HTMLElement, values: Partial<Record<FlooredKey, unknown>>): void => {
    for (const [key, cssVar] of FLOORED_VARS) {
        const value = values[key];
        if (typeof value !== 'string' || !value) continue;
        if (root.style.getPropertyValue(cssVar) !== value) root.style.setProperty(cssVar, value);
    }
};

const askRustFloors = (root: HTMLElement, themeId: string, painted: ThemePalette, fallback: Record<FlooredKey, string>): void => {
    const seq = ++floorSeq;
    // Wrapped so a missing Tauri bridge (tests, the harness) that throws
    // instead of rejecting lands in the same catch.
    Promise.resolve()
        .then(() => invoke<Partial<Record<FlooredKey, unknown>>>('lift_theme_palette', { palette: painted }))
        .then((lifted) => {
            if (seq !== floorSeq) return;
            paintFloors(root, lifted ?? {});
            flooredThemeId = themeId;
        })
        .catch(() => {
            // No answer: the palette's own values stand (a held repaint still
            // shows the previous answer, so put this palette's values back).
            if (seq === floorSeq) paintFloors(root, fallback);
        });
};

// Apply theme to CSS variables. `floor` is what sits under the page; only a
// window that is itself see-through (the Windows chat overlay, whose slab
// alpha is its own slider) passes null to keep the theme's alpha.
export const applyTheme = (
    theme: Theme,
    { floor = IS_MOBILE ? PHONE_FLOOR : WINDOW_FLOOR }: { floor?: string | null } = {},
): void => {
    const root = document.documentElement;
    const { palette } = theme;

    // Core colors, with the floors made solid (see opaqueFloors).
    const floors = floor
        ? opaqueFloors(palette.background, palette.backgroundTertiary, floor)
        : { background: palette.background, tertiary: palette.backgroundTertiary };
    root.style.setProperty('--color-background', floors.background);
    root.style.setProperty('--color-background-secondary', palette.backgroundSecondary);
    root.style.setProperty('--color-background-tertiary', floors.tertiary);

    // Surface colors. Capture the raw per-theme tints, then paint them at the
    // current glassiness via writeSurfaceGlass so switching themes never flashes
    // full glass when the user has dialled it down (and a reduced level carries
    // straight over to the new palette). applyGlassStrength repaints on slider move.
    activeSurfaceTints = {
        surface: palette.surface,
        surfaceHover: palette.surfaceHover,
        surfaceActive: palette.surfaceActive,
        tertiary: floors.tertiary,
    };
    writeSurfaceGlass(effectiveGlassStrength());

    // The floored colors (see askRustFloors). A different theme paints its own
    // values at once; a repaint of the theme already on screen keeps Rust's.
    const holdFloors = theme.id === flooredThemeId;
    if (!holdFloors) flooredThemeId = null;

    // Text colors
    root.style.setProperty('--color-text-primary', palette.textPrimary);
    if (!holdFloors) {
        root.style.setProperty('--color-text-secondary', palette.textSecondary);
        root.style.setProperty('--color-text-muted', palette.textMuted);
    }

    // Accent colors
    root.style.setProperty('--color-accent', palette.accent);
    root.style.setProperty('--color-accent-hover', palette.accentHover);
    root.style.setProperty('--color-accent-muted', palette.accentMuted);
    if (!holdFloors) {
        root.style.setProperty('--color-accent-on-surface', palette.accentOnSurface ?? palette.accent);
    }
    // Generate neon variant by lightening accent by 30%
    root.style.setProperty('--color-accent-neon', lightenColor(palette.accent, 30));
    // Raw accent channels for rgba(var(--color-accent-rgb), a) shadows/halos.
    const accentRgb = hexToRgb(parseColorToHex(palette.accent));
    root.style.setProperty('--color-accent-rgb', `${accentRgb.r}, ${accentRgb.g}, ${accentRgb.b}`);

    // Border colors
    root.style.setProperty('--color-border', palette.border);
    root.style.setProperty('--color-border-light', palette.borderLight);
    root.style.setProperty('--color-border-subtle', palette.borderSubtle);

    // Semantic colors
    root.style.setProperty('--color-success', palette.success);
    root.style.setProperty('--color-warning', palette.warning);
    root.style.setProperty('--color-error', palette.error);
    root.style.setProperty('--color-info', palette.info);

    // Scrollbar colors
    root.style.setProperty('--color-scrollbar-thumb', palette.scrollbarThumb);
    root.style.setProperty('--color-scrollbar-track', palette.scrollbarTrack);

    // Glass effect opacities
    root.style.setProperty('--glass-opacity', palette.glassOpacity);
    root.style.setProperty('--glass-hover-opacity', palette.glassHoverOpacity);
    root.style.setProperty('--glass-active-opacity', palette.glassActiveOpacity);

    // Highlight colors
    root.style.setProperty('--color-highlight-pink', palette.highlight.pink);
    root.style.setProperty('--color-highlight-purple', palette.highlight.purple);
    root.style.setProperty('--color-highlight-blue', palette.highlight.blue);
    root.style.setProperty('--color-highlight-cyan', palette.highlight.cyan);
    root.style.setProperty('--color-highlight-green', palette.highlight.green);
    root.style.setProperty('--color-highlight-yellow', palette.highlight.yellow);
    root.style.setProperty('--color-highlight-orange', palette.highlight.orange);
    root.style.setProperty('--color-highlight-red', palette.highlight.red);

    // Store theme id on body for potential CSS-based theme detection
    document.body.setAttribute('data-theme', theme.id);

    // Treatments beyond the palette (Prism's refracted light) key off <html>,
    // so every window that applies a theme picks them up. Cleared explicitly:
    // switching from Prism to a palette-only theme must drop the light.
    if (theme.effect) root.setAttribute('data-theme-effect', theme.effect);
    else root.removeAttribute('data-theme-effect');
    // Announce the palette so platform chrome can follow it.
    //
    // Android draws the app BEHIND the status and navigation bars, so nothing
    // but the active palette knows what is under the clock and the battery
    // icon. The mobile shell listens for this and sets the bar icon colour
    // (src/mobile/systemBars.ts). Deliberately an event rather than a direct
    // call: this file is shared, and shared code must not reach for a native
    // bridge that only exists on the phone.
    themeDark = needsLightContentOn(floors.background);
    root.dataset.themeTone = themeDark ? 'dark' : 'light';
    window.dispatchEvent(
        new CustomEvent('sn:theme-applied', {
            detail: {
                background: floors.background,
                lightContent: needsLightContentOn(floors.background),
            },
        }),
    );

    // Rust measures against the floors as painted (a see-through custom
    // background flattened over what is under the page), not the raw alpha.
    askRustFloors(
        root,
        theme.id,
        { ...palette, background: floors.background, backgroundTertiary: floors.tertiary },
        {
            accentOnSurface: palette.accentOnSurface ?? palette.accent,
            textMuted: palette.textMuted,
            textSecondary: palette.textSecondary,
        },
    );
};

// Default glassiness (percent). 100 = full frosted glass — the look every
// theme is tuned around.
export const DEFAULT_GLASS_TRANSPARENCY = 100;

// Apply the global glassiness to the live document. `transparency` is a percent
// (0-100): 100 is full frosted glass (the signature look every theme is tuned
// around), 0 is a completely flat, solid, blur-free interface. Writes
// --glass-strength (read by the blur scaling in globals.css), repaints the
// surface tints toward solid, and flags data-glass="off" at 0 so the CSS floor
// strips any remaining backdrop blur. Kept separate from applyTheme so switching
// themes never resets the user's chosen glassiness.
export const applyGlassStrength = (transparency: number): void => {
    const clamped = Math.max(0, Math.min(100, transparency)) / 100;
    lastGlassStrength = clamped;
    const root = document.documentElement;
    const strength = effectiveGlassStrength();
    root.style.setProperty('--glass-strength', String(strength));
    writeSurfaceGlass(strength);
    root.setAttribute('data-glass', clamped === 0 ? 'off' : 'on');
};

// The default when a settings object carries no `glass_blur`. Rust fills the
// field with the same default (on, every platform) on every load, so this only
// covers a caller holding a settings object built without it.
export const DEFAULT_GLASS_BLUR = true;

// Turn the backdrop blur behind glass on or off. Off flags data-blur="off",
// which strips every backdrop-filter in globals.css, and repaints the surfaces
// at the capped strength so the glass stays readable without frost.
export const applyGlassBlur = (enabled: boolean): void => {
    document.documentElement.setAttribute('data-blur', enabled ? 'on' : 'off');
    // Blur starts on, and stays on everywhere but Linux: repaint only on a change.
    if (enabled === glassBlurEnabled) return;
    glassBlurEnabled = enabled;
    applyGlassStrength(lastGlassStrength * 100);
};

// ─── App font ───────────────────────────────────────────────────────────────
// User-selectable interface font, chosen in Theme > Font. Like glassiness, it's
// independent of the color palette, so switching themes never resets it. The
// chosen stack is written to --app-font on :root; body and the Tailwind
// `font-sans` utility both read that variable (see globals.css / tailwind config).

export type FontId = 'satoshi' | 'twitch' | 'geist' | 'manrope' | 'outfit' | 'space-grotesk' | 'serif' | 'system' | 'custom';

export interface FontOption {
    id: FontId;
    label: string;
    description: string;
    /** font-family stack written to --app-font (and used for in-card previews). */
    stack: string;
    /**
     * Chat message body weight for this font, written to --chat-body-weight.
     * Denser faces (Inter) read heavier at a given weight, so they go lighter
     * here to keep chat looking lean. Defaults to 300 when omitted.
     */
    chatWeight?: number;
}

export const FONT_OPTIONS: FontOption[] = [
    {
        id: 'satoshi',
        label: 'Satoshi',
        description: 'The StreamNook house font. Clean, geometric sans.',
        stack: '"Satoshi", -apple-system, BlinkMacSystemFont, sans-serif',
    },
    {
        id: 'twitch',
        label: 'Twitch',
        description: 'Inter — the open-source typeface Twitch uses across its UI and chat.',
        stack: '"Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
        // Inter is denser than Satoshi, so chat at the normal 300 reads heavy.
        // Drop to 200 (ExtraLight) so chat stays lean under the Twitch font.
        chatWeight: 200,
    },
    {
        id: 'geist',
        label: 'Geist',
        description: 'A clean, modern geometric sans. Sleek and lean at any size.',
        stack: '"Geist", -apple-system, BlinkMacSystemFont, sans-serif',
    },
    {
        id: 'manrope',
        label: 'Manrope',
        description: 'Modern geometric sans, slightly narrow. Light and lean for dense UI.',
        stack: '"Manrope", -apple-system, BlinkMacSystemFont, sans-serif',
    },
    {
        id: 'outfit',
        label: 'Outfit',
        description: 'Uniform, minimal geometric sans. The most stripped-down of the set.',
        stack: '"Outfit", -apple-system, BlinkMacSystemFont, sans-serif',
    },
    {
        id: 'space-grotesk',
        label: 'Space Grotesk',
        description: 'Geometric sans with a little more character, still lean.',
        stack: '"Space Grotesk", -apple-system, BlinkMacSystemFont, sans-serif',
    },
    {
        id: 'serif',
        label: 'Serif',
        description: 'Fraunces — a soft, characterful serif for an editorial feel.',
        stack: '"Fraunces Variable", Georgia, "Times New Roman", serif',
    },
    {
        id: 'system',
        label: 'System',
        description: "Your device's native font. Fast and familiar.",
        stack: 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
    },
];

export const DEFAULT_FONT_ID: FontId = 'satoshi';

const FONT_BY_ID: Record<string, FontOption> = Object.fromEntries(
    FONT_OPTIONS.map((o) => [o.id, o]),
);

// Default chat message body weight (most fonts). Inter overrides lighter.
export const DEFAULT_CHAT_BODY_WEIGHT = 300;

// ─── Custom font ────────────────────────────────────────────────────────────
// A user-typed family name, stored as settings.font = 'custom' plus
// settings.font_custom = the bare name. Kept out of FONT_OPTIONS so FONT_BY_ID
// never holds a fake stack and applyFont's unknown-id fallback still means
// "unknown".
export const CUSTOM_FONT_ID: FontId = 'custom';

// Popular free families offered as one-click fills under the text input.
export const SUGGESTED_FONTS: string[] = [
    'Poppins', 'Bebas Neue', 'Rubik', 'Lato',
    'Nunito', 'Playfair Display', 'JetBrains Mono', 'Quicksand',
];

// Strip anything that would let a typed name break out of its slot in the
// font-family stack (quotes, commas, semicolons, braces, parens, backslash)
// and cap the length. What survives is a plain family name.
export const sanitizeFontFamily = (raw: string): string =>
    (raw || '').replace(/["',;{}()\\]/g, '').trim().slice(0, 64);

// Same fallback tail the bundled options use, so an unresolvable name still
// lands somewhere readable instead of blanking the UI.
export const customFontStack = (family: string): string =>
    `"${family}", -apple-system, BlinkMacSystemFont, sans-serif`;

// Families that never need a network fetch: CSS generics, and the faces this
// app already ships as local woff2 (see the @font-face blocks in globals.css).
const NO_FETCH_FAMILIES = new Set([
    'system-ui', 'ui-sans-serif', 'ui-serif', 'ui-monospace', 'sans-serif',
    'serif', 'monospace', 'cursive', 'fantasy', '-apple-system',
    'blinkmacsystemfont', 'segoe ui', 'roboto', 'inherit', 'initial', 'unset',
    'satoshi', 'inter', 'geist', 'manrope', 'outfit', 'space grotesk',
    'fraunces variable',
]);

const WEBFONT_LINK_ID = 'sn-app-webfont';
const WEBFONT_DEBOUNCE_MS = 250;
const FONT_READY_TIMEOUT_MS = 1500;
// The UI spans 200-800. Bunny returns only the weights a family actually has
// (measured: Bebas Neue with this list answers 200 with just its 400 face), and
// the browser downloads only the faces whose weight and unicode-range are used,
// so asking wide is free.
const APP_FONT_WEIGHTS = ':wght@200;300;400;500;600;700;800';

// Bunny Fonts is a drop-in Google Fonts mirror with the same css2 API and
// catalog. It is used instead of fonts.googleapis.com because WebView2's
// tracking prevention blocks the Google host, so the app would render nothing
// while a plain browser worked (same reason the overlay uses it).
const bunnyHref = (family: string, weights = ''): string =>
    `https://fonts.bunny.net/css2?family=${encodeURIComponent(family).replace(/%20/g, '+')}${weights}&display=swap`;

// Wait until the font engine can actually draw the family, then run `done`.
// Bounded by a timeout, and `done` runs on every path (resolved, rejected,
// timed out) so a slow or unreachable CDN can never strand the UI on the old
// font. A family with no matching @font-face resolves with an empty match list
// rather than rejecting, which covers both typos and locally-installed fonts.
const whenFamilyReady = (family: string, done: () => void): void => {
    let settled = false;
    const finish = () => { if (!settled) { settled = true; done(); } };
    const timer = setTimeout(finish, FONT_READY_TIMEOUT_MS);
    const clear = () => clearTimeout(timer);
    try {
        document.fonts.load(`400 1em "${family}"`)
            .then(() => { clear(); finish(); })
            .catch(() => { clear(); finish(); });
    } catch {
        clear();
        finish();
    }
};

let webfontTimer: ReturnType<typeof setTimeout> | null = null;
let loadedFamily = '';

// Load a typed family, then call back once it is usable. ONE <link> is reused
// and its href swapped, so switching fonts never leaves dead stylesheets
// behind, and the fetch is debounced so typing a name does not fire a request
// per character.
export const loadWebFont = (family: string, onReady?: () => void): void => {
    if (typeof document === 'undefined') { onReady?.(); return; }
    // Cancel any queued load FIRST, before the dedupe check below can return.
    // Otherwise a pending request for an earlier prefix of what is now being
    // typed survives and wins the race, leaving the wrong family loaded.
    if (webfontTimer) { clearTimeout(webfontTimer); webfontTimer = null; }
    const lower = family.toLowerCase();
    if (!family || NO_FETCH_FAMILIES.has(lower) || lower === loadedFamily) {
        onReady?.();
        return;
    }
    webfontTimer = setTimeout(() => {
        webfontTimer = null;
        loadedFamily = lower;
        let link = document.getElementById(WEBFONT_LINK_ID) as HTMLLinkElement | null;
        if (!link) {
            link = document.createElement('link');
            link.id = WEBFONT_LINK_ID;
            link.rel = 'stylesheet';
            document.head.appendChild(link);
        }
        const el = link;
        el.onload = () => whenFamilyReady(family, () => onReady?.());
        // A network failure is the only thing that fires this (a bad family
        // name is a 200 with an error comment in the body). Commit anyway so
        // an offline launch still resolves rather than hanging.
        el.onerror = () => onReady?.();
        el.href = bunnyHref(family, APP_FONT_WEIGHTS);
    }, WEBFONT_DEBOUNCE_MS);
};

// Preview faces for the suggestion list. One combined request, no weight axis,
// fired once. A bad entry does not poison the batch.
export const loadSuggestionPreviews = (): void => {
    if (typeof document === 'undefined') return;
    const id = 'sn-app-font-suggestions';
    if (document.getElementById(id)) return;
    const link = document.createElement('link');
    link.id = id;
    link.rel = 'stylesheet';
    link.href = 'https://fonts.bunny.net/css2?'
        + SUGGESTED_FONTS.map((f) => `family=${encodeURIComponent(f).replace(/%20/g, '+')}`).join('&')
        + '&display=swap';
    document.head.appendChild(link);
};

// Bumped on every call so an in-flight custom-font commit that has been
// superseded (user kept typing, or picked a preset) is discarded instead of
// landing late over the newer choice.
let fontApplyToken = 0;

// Apply the chosen interface font to the live document. Unknown ids fall back
// to the default so a stale/garbage setting can never blank the font. Also sets
// --chat-body-weight so chat message text can render lighter under denser faces.
export const applyFont = (fontId: string | undefined, customFamily?: string): void => {
    const root = document.documentElement;
    const token = ++fontApplyToken;

    if (fontId === CUSTOM_FONT_ID) {
        const family = sanitizeFontFamily(customFamily ?? '');
        // An empty custom name falls through to the default rather than writing
        // an empty stack, so the UI can never end up unstyled.
        if (family) {
            // Hold the current font up until the face is actually usable, then
            // swap once. Writing --app-font first would drop the whole UI to a
            // system fallback for the length of the request, so every launch
            // would flash twice.
            loadWebFont(family, () => {
                if (token !== fontApplyToken) return;
                root.style.setProperty('--app-font', customFontStack(family));
                root.style.setProperty('--chat-body-weight', String(DEFAULT_CHAT_BODY_WEIGHT));
            });
            return;
        }
    }

    const opt = FONT_BY_ID[fontId ?? DEFAULT_FONT_ID] ?? FONT_BY_ID[DEFAULT_FONT_ID];
    root.style.setProperty('--app-font', opt.stack);
    root.style.setProperty('--chat-body-weight', String(opt.chatWeight ?? DEFAULT_CHAT_BODY_WEIGHT));
};
