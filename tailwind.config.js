// CSS-variable color that supports opacity modifiers (bg-accent/20). A plain
// 'var(...)' string silently generates NO css for modifier classes, which left
// 300+ authored token tints dead until 2026-07-26.
const varColor = (cssVar) => ({ opacityValue }) =>
  opacityValue === undefined
    ? `var(${cssVar})`
    : `color-mix(in srgb, var(${cssVar}) calc(${opacityValue} * 100%), transparent)`;

// Glassiness 0% must leave no surface see-through. Two helpers carry that:
//
// underColor is every BACKGROUND colour's modifier form. It mixes the tint over
// `--glass-under` instead of over transparent. Only a glass class sets that
// variable, to its own opaque base faded in as the slider drops (globals.css,
// "Glass under"). So a hover:bg-white/10 or !bg-red-500/15 on a glass button
// tints the solid base at 0% instead of replacing it, and everywhere else, and
// at full glass, the result is exactly the old one. Backgrounds only: a text,
// border, ring or shadow colour paints over something else, where an opaque
// base would show as a halo.
//
// glassColor is the `glass` family (bg-glass-ink/90, bg-glass-raised/95...):
// the modifier is the alpha at full glass, ramping to fully opaque at 0%. A
// floating surface that is not a glass class takes its fill from here; `/0`
// is clear at full glass and solid at 0%.
const underColor = (color) => ({ opacityValue }) =>
  opacityValue === undefined
    ? color
    : `color-mix(in srgb, ${color} calc(${opacityValue} * 100%), var(--glass-under, transparent))`;
const glassColor = (color) => ({ opacityValue }) =>
  opacityValue === undefined
    ? color
    : `color-mix(in srgb, ${color} calc((${opacityValue} + (1 - ${opacityValue}) * (1 - var(--glass-strength, 1))) * 100%), transparent)`;

// Theme colours by name, so the text/border family (varColor) and the
// background family (underColor) are built from one list.
const THEME_COLORS = {
  background: 'var(--color-background)',
  secondary: 'var(--color-background-secondary)',
  tertiary: 'var(--color-background-tertiary)',
  accent: 'var(--color-accent)',
  'accent-hover': 'var(--color-accent-hover)',
  'accent-muted': 'var(--color-accent-muted)',
  'accent-neon': 'var(--color-accent-neon)',
  textPrimary: 'var(--color-text-primary)',
  textSecondary: 'var(--color-text-secondary)',
  textMuted: 'var(--color-text-muted)',
  border: 'var(--color-border)',
  borderLight: 'var(--color-border-light)',
  borderSubtle: 'var(--color-border-subtle)',
  surface: 'var(--color-surface)',
  'surface-hover': 'var(--color-surface-hover)',
  'surface-active': 'var(--color-surface-active)',
  success: 'var(--color-success)',
  warning: 'var(--color-warning)',
  error: 'var(--color-error)',
  info: 'var(--color-info)',
  live: 'var(--color-live)',
};
const HIGHLIGHT_COLORS = ['pink', 'purple', 'blue', 'cyan', 'green', 'yellow', 'orange', 'red'];
const GLASS_COLORS = {
  DEFAULT: 'var(--color-surface)',
  hover: 'var(--color-surface-hover)',
  active: 'var(--color-surface-active)',
  base: 'var(--color-background)',
  raised: 'var(--color-background-tertiary)',
  ink: '#09090b',
};

// Tailwind's own palette, for backgrounds. The deprecated names are getters
// that print a warning when read, so they are skipped.
const DEPRECATED = new Set(['lightBlue', 'warmGray', 'trueGray', 'coolGray', 'blueGray']);
const KEYWORDS = new Set(['inherit', 'current', 'transparent']);
const palette = require('tailwindcss/colors');
const underPalette = {};
for (const name of Object.keys(palette)) {
  if (DEPRECATED.has(name) || KEYWORDS.has(name)) continue;
  const value = palette[name];
  underPalette[name] =
    typeof value === 'string'
      ? underColor(value)
      : Object.fromEntries(Object.entries(value).map(([shade, hex]) => [shade, underColor(hex)]));
}

const mapValues = (obj, fn) => Object.fromEntries(Object.entries(obj).map(([k, v]) => [k, fn(v)]));

/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./src/**/*.{html,js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      screens: {
        '3xl': '1920px',
      },
      // Theme-aware colors using CSS variables. `live` is the LIVE indicator
      // red, pinned in globals.css and deliberately not themed.
      colors: {
        ...mapValues(THEME_COLORS, (v) => varColor(v.slice(4, -1))),
        glass: mapValues(GLASS_COLORS, glassColor),
        highlight: Object.fromEntries(
          HIGHLIGHT_COLORS.map((h) => [h, varColor(`--color-highlight-${h}`)]),
        ),
      },
      // The same names for backgrounds, mixed over --glass-under (see above).
      backgroundColor: {
        ...underPalette,
        ...mapValues(THEME_COLORS, underColor),
        glass: mapValues(GLASS_COLORS, glassColor),
        highlight: Object.fromEntries(
          HIGHLIGHT_COLORS.map((h) => [h, underColor(`var(--color-highlight-${h})`)]),
        ),
      },
      fontFamily: {
        // Driven by --app-font (set by applyFont in themes/index.ts) so the
        // `font-sans` utility follows the user's Theme > Font choice. The
        // static entries are fallbacks if the variable is ever unset.
        sans: ['var(--app-font)', 'Satoshi', '-apple-system', 'BlinkMacSystemFont', 'sans-serif'],
      },
      backdropBlur: {
        xs: '2px',
      },
      keyframes: {
        'fade-in': {
          '0%': { opacity: '0', transform: 'translateY(-4px)' },
          '100%': { opacity: '1', transform: 'translateY(0)' },
        },
        // Gentle fade + slight grow for overlays/modals so they ease in
        // instead of snapping. Pairs with the house spring feel.
        'scale-in': {
          '0%': { opacity: '0', transform: 'scale(0.96)' },
          '100%': { opacity: '1', transform: 'scale(1)' },
        },
        shimmer: {
          '0%': { backgroundPosition: '-200% 0' },
          '100%': { backgroundPosition: '200% 0' },
        },
        droplet: {
          '0%, 100%': {
            transform: 'translateY(-8px)',
            opacity: '0'
          },
          '20%': {
            transform: 'translateY(-8px)',
            opacity: '1'
          },
          '40%': {
            transform: 'translateY(-8px)',
            opacity: '1'
          },
          '60%': {
            transform: 'translateY(8px)',
            opacity: '1'
          },
          '80%': {
            transform: 'translateY(12px)',
            opacity: '0'
          },
        },
        splash: {
          '0%': {
            transform: 'scale(0.5)',
            opacity: '1'
          },
          '50%': {
            transform: 'scale(1.5)',
            opacity: '0.6'
          },
          '100%': {
            transform: 'scale(2.5)',
            opacity: '0'
          },
        },
        ripple: {
          '0%, 56%': {
            transform: 'scale(0.8)',
            opacity: '0'
          },
          '60%': {
            transform: 'scale(1.0)',
            opacity: '0.5'
          },
          '68%': {
            transform: 'scale(1.3)',
            opacity: '0.4'
          },
          '78%': {
            transform: 'scale(1.6)',
            opacity: '0.2'
          },
          '88%': {
            transform: 'scale(1.9)',
            opacity: '0.1'
          },
          '100%': {
            transform: 'scale(2.2)',
            opacity: '0'
          },
        },
      },
      animation: {
        'fade-in': 'fade-in 0.3s ease-out forwards',
        'scale-in': 'scale-in 0.2s cubic-bezier(0.2, 0.8, 0.2, 1) forwards',
        shimmer: 'shimmer 2s ease-in-out infinite',
        droplet: 'droplet 2.5s ease-in-out infinite',
        splash: 'splash 0.6s ease-out forwards',
        ripple: 'ripple 2.5s ease-out infinite',
      },
    },
  },
  plugins: [],
}
