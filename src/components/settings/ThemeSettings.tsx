import { useState, useEffect, useRef } from 'react';
import { useAppStore } from '../../stores/AppStore';
import { themes, themeCategories, getThemeById, applyTheme, customThemeToTheme, getThemeByIdWithCustom, applyGlassStrength, DEFAULT_GLASS_TRANSPARENCY, applyFont, FONT_OPTIONS, DEFAULT_FONT_ID, CUSTOM_FONT_ID, SUGGESTED_FONTS, customFontStack, sanitizeFontFamily, loadSuggestionPreviews, Theme, OLED_THEME_ID, DEFAULT_OLED_ACCENT, OLED_ACCENT_PRESETS, getOledTheme } from '../../themes';
import { Check, Palette, Sparkles, Moon, Leaf, Code, Star, Plus, Edit2, PaintBucket, Droplets, Type } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import ThemeCreator from './ThemeCreator';
import ThemeColorPicker from '../ThemeColorPicker';
import type { CustomTheme } from '../../types';

const getCategoryIcon = (categoryId: string) => {
    switch (categoryId) {
        case 'signature':
            return <Star size={16} />;
        case 'universal':
            return <Sparkles size={16} />;
        case 'modern':
            return <Moon size={16} />;
        case 'classic':
            return <Code size={16} />;
        case 'cozy':
            return <Leaf size={16} />;
        default:
            return <Palette size={16} />;
    }
};

interface ThemeCardProps {
    theme: Theme;
    isSelected: boolean;
    onSelect: () => void;
    isCustom?: boolean;
    onEdit?: () => void;
}

const ThemeCard = ({ theme, isSelected, onSelect, isCustom, onEdit }: ThemeCardProps) => {
    const { palette } = theme;

    return (
        <button
            onClick={onSelect}
            className={`
                relative w-full p-3 rounded-lg transition-all duration-200
                border text-left group
                ${isSelected
                    ? 'border-accent ring-2 ring-accent/30'
                    : 'border-borderSubtle hover:border-borderLight'
                }
            `}
            style={{
                backgroundColor: palette.background,
            }}
        >
            {/* Color Preview */}
            <div className="flex gap-1.5 mb-2">
                <Tooltip content="Accent" side="top">
                <div
                    className="w-4 h-4 rounded-full"
                    style={{ backgroundColor: palette.accent }}
                />
                </Tooltip>
                <Tooltip content="Pink" side="top">
                <div
                    className="w-4 h-4 rounded-full"
                    style={{ backgroundColor: palette.highlight.pink }}
                />
                </Tooltip>
                <Tooltip content="Purple" side="top">
                <div
                    className="w-4 h-4 rounded-full"
                    style={{ backgroundColor: palette.highlight.purple }}
                />
                </Tooltip>
                <Tooltip content="Blue" side="top">
                <div
                    className="w-4 h-4 rounded-full"
                    style={{ backgroundColor: palette.highlight.blue }}
                />
                </Tooltip>
                <Tooltip content="Green" side="top">
                <div
                    className="w-4 h-4 rounded-full"
                    style={{ backgroundColor: palette.highlight.green }}
                />
                </Tooltip>
            </div>

            {/* Theme Info */}
            <div className="mb-1 flex items-center justify-between">
                <h4
                    className="font-semibold text-sm"
                    style={{ color: palette.textPrimary }}
                >
                    {theme.name}
                </h4>
                {isCustom && onEdit && (
                    <Tooltip content="Edit theme" side="top">
                    <button
                        onClick={(e) => {
                            e.stopPropagation();
                            onEdit();
                        }}
                        className="p-1 rounded opacity-0 group-hover:opacity-100 transition-opacity hover:bg-white/10"
                    >
                        <Edit2 size={12} style={{ color: palette.textSecondary }} />
                    </button>
                    </Tooltip>
                )}
            </div>

            <p
                className="text-xs leading-relaxed line-clamp-2"
                style={{ color: palette.textSecondary }}
            >
                {theme.description}
            </p>

            {/* Selection Indicator */}
            {isSelected && (
                <div className="absolute top-2 right-2 w-5 h-5 rounded-full bg-accent flex items-center justify-center">
                    <Check size={12} className="text-background" strokeWidth={3} />
                </div>
            )}

            {/* Custom Badge */}
            {isCustom && (
                <div className="absolute bottom-2 right-2 px-1.5 py-0.5 rounded text-[10px] font-medium bg-accent/20 text-accent">
                    Custom
                </div>
            )}

            {/* Hover effect overlay */}
            <div
                className="absolute inset-0 rounded-lg opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none"
                style={{
                    background: `linear-gradient(135deg, ${palette.accent}10 0%, transparent 50%)`,
                }}
            />
        </button>
    );
};

const ThemeSettings = () => {
    const { settings, updateSettings } = useAppStore();
    const currentThemeId = settings.theme || 'winters-glass';
    const customThemes = settings.custom_themes || [];
    
    const [isCreating, setIsCreating] = useState(false);
    const [editingTheme, setEditingTheme] = useState<CustomTheme | undefined>(undefined);

    const glassTransparency = settings.glass_transparency ?? DEFAULT_GLASS_TRANSPARENCY;

    // Track the slider locally so the thumb follows the cursor instantly. The
    // persisted value only catches up on release, so we mirror it here for the
    // controlled input and pick up any changes made from another window.
    const [liveGlass, setLiveGlass] = useState(glassTransparency);
    // Adopt a persisted change (a release here, or another window) during
    // render so the thumb never shows a stale value for a commit.
    const [seenGlass, setSeenGlass] = useState(glassTransparency);
    if (glassTransparency !== seenGlass) {
        setSeenGlass(glassTransparency);
        setLiveGlass(glassTransparency);
    }

    // While dragging: only move the thumb and repaint via the CSS variable (cheap).
    // No disk write — persisting on every tick is what made the slider stutter.
    const handleGlassInput = (value: number) => {
        setLiveGlass(value);
        applyGlassStrength(value);
    };

    // On release: persist once. updateSettings writes to disk, broadcasts to other
    // windows, and re-renders the whole app, so it must not run mid-drag.
    const commitGlass = (value: number) => {
        if (value === glassTransparency) return;
        updateSettings({ ...settings, glass_transparency: value });
    };

    // OLED is the one signature theme with a user-chosen accent. Mirror the saved
    // value locally so the swatches, picker, and preview track the cursor live
    // while the persist is deferred (same approach as the Glassiness slider).
    const oledAccent = settings.oled_accent ?? DEFAULT_OLED_ACCENT;
    const [liveOledAccent, setLiveOledAccent] = useState(oledAccent);
    const [seenOledAccent, setSeenOledAccent] = useState(oledAccent);
    if (oledAccent !== seenOledAccent) {
        setSeenOledAccent(oledAccent);
        setLiveOledAccent(oledAccent);
    }
    const oledPersistTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Get current theme (could be custom or built-in). For OLED, fold in the
    // chosen accent so the header chip and preview reflect it.
    const currentTheme = currentThemeId === OLED_THEME_ID
        ? getOledTheme(liveOledAccent)
        : getThemeByIdWithCustom(currentThemeId, customThemes);

    const currentFontId = settings.font || DEFAULT_FONT_ID;
    const isCustomFont = currentFontId === CUSTOM_FONT_ID;
    // Local mirror so typing stays responsive; persisted on a debounce below.
    const [customFontDraft, setCustomFontDraft] = useState(settings.font_custom ?? '');
    const customFontTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Suggestion previews are a network fetch, so only pay for it once the
    // custom section is actually open.
    useEffect(() => {
        if (isCustomFont) loadSuggestionPreviews();
    }, [isCustomFont]);

    // Keep the latest draft reachable from the unmount cleanup below, which
    // closes over the first render's values.
    const customFontDraftRef = useRef(customFontDraft);
    useEffect(() => { customFontDraftRef.current = customFontDraft; }, [customFontDraft]);

    // FLUSH a pending write on unmount, never drop it. Closing Settings inside
    // the debounce window must not lose the font the user just typed: it was
    // already applied live, so silently failing to save it means it vanishes on
    // the next launch with no sign anything went wrong.
    useEffect(() => () => {
        if (!customFontTimer.current) return;
        clearTimeout(customFontTimer.current);
        customFontTimer.current = null;
        const store = useAppStore.getState();
        const pending = customFontDraftRef.current;
        if (store.settings.font !== CUSTOM_FONT_ID || store.settings.font_custom !== pending) {
            void store.updateSettings({ ...store.settings, font: CUSTOM_FONT_ID, font_custom: pending });
        }
    }, []);

    const handleThemeChange = (themeId: string) => {
        // OLED resolves through its chosen accent rather than the static entry.
        const theme = themeId === OLED_THEME_ID
            ? getOledTheme(settings.oled_accent)
            : getThemeByIdWithCustom(themeId, customThemes);
        if (theme) {
            // Apply theme immediately
            applyTheme(theme);
            // Save to settings
            updateSettings({ ...settings, theme: themeId });
        }
    };

    // Apply a new OLED accent live (cheap CSS-var repaint), then persist once the
    // user settles so dragging the picker doesn't write to disk + re-render the
    // whole app on every tick.
    const handleOledAccentChange = (accentHex: string) => {
        setLiveOledAccent(accentHex);
        if (currentThemeId === OLED_THEME_ID) {
            applyTheme(getOledTheme(accentHex));
        }
        if (oledPersistTimer.current) clearTimeout(oledPersistTimer.current);
        oledPersistTimer.current = setTimeout(() => {
            // Read the freshest settings at fire time (not the closed-over copy):
            // the user may have changed another setting — e.g. switched themes —
            // during the debounce window, and spreading a stale copy would undo it.
            const latest = useAppStore.getState().settings;
            updateSettings({ ...latest, oled_accent: accentHex });
        }, 250);
    };

    const handleFontChange = (fontId: string) => {
        if (fontId === currentFontId) return;
        // Drop any pending custom-font write. Without this, picking a preset
        // within the debounce window gets silently overwritten a moment later
        // by the queued { font: 'custom' } save.
        if (customFontTimer.current) {
            clearTimeout(customFontTimer.current);
            customFontTimer.current = null;
        }
        // Apply live, then persist (mirrors theme + glassiness flow).
        applyFont(fontId, fontId === CUSTOM_FONT_ID ? customFontDraft : undefined);
        updateSettings({ ...useAppStore.getState().settings, font: fontId });
    };

    // Ask for the font on every keystroke (the fetch is debounced and the swap
    // only commits once the face is usable, so partial names never reach the
    // UI), but write settings only once the user settles. Reads the freshest
    // settings at fire time for the same reason handleOledAccentChange does:
    // another setting may have changed during the window, and spreading a
    // stale copy would undo it.
    const handleCustomFontChange = (family: string) => {
        const clean = sanitizeFontFamily(family);
        setCustomFontDraft(clean);
        applyFont(CUSTOM_FONT_ID, clean);
        if (customFontTimer.current) clearTimeout(customFontTimer.current);
        customFontTimer.current = setTimeout(() => {
            customFontTimer.current = null;
            const latest = useAppStore.getState().settings;
            updateSettings({ ...latest, font: CUSTOM_FONT_ID, font_custom: clean });
        }, 400);
    };

    const handleSaveCustomTheme = (theme: CustomTheme) => {
        const existingIndex = customThemes.findIndex((t) => t.id === theme.id);
        let newCustomThemes: CustomTheme[];

        if (existingIndex >= 0) {
            // Update existing
            newCustomThemes = [...customThemes];
            newCustomThemes[existingIndex] = theme;
        } else {
            // Add new
            newCustomThemes = [...customThemes, theme];
        }

        // Apply the theme
        applyTheme(customThemeToTheme(theme));

        // Save to settings
        updateSettings({
            ...settings,
            theme: theme.id,
            custom_themes: newCustomThemes,
        });

        setIsCreating(false);
        setEditingTheme(undefined);
    };

    const handleDeleteCustomTheme = (themeId: string) => {
        const newCustomThemes = customThemes.filter((t) => t.id !== themeId);
        
        // If the deleted theme was active, switch to default
        const newActiveTheme = currentThemeId === themeId ? 'winters-glass' : currentThemeId;
        const themeToApply = getThemeById(newActiveTheme) || themes[0];
        applyTheme(themeToApply);

        updateSettings({
            ...settings,
            theme: newActiveTheme,
            custom_themes: newCustomThemes,
        });

        setIsCreating(false);
        setEditingTheme(undefined);
    };

    const handleEditTheme = (theme: CustomTheme) => {
        setEditingTheme(theme);
        setIsCreating(true);
    };

    // Show creator view
    if (isCreating) {
        return (
            <ThemeCreator
                editingTheme={editingTheme}
                onClose={() => {
                    setIsCreating(false);
                    setEditingTheme(undefined);
                }}
                onSave={handleSaveCustomTheme}
                onDelete={editingTheme ? handleDeleteCustomTheme : undefined}
            />
        );
    }

    return (
        <div className="space-y-5">
            {/* Header Row */}
            <div className="flex items-center justify-between gap-4">
                <div className="flex items-center gap-2">
                    <Palette size={18} className="text-accent" />
                    <h3 className="text-base font-semibold text-textPrimary">Theme</h3>
                </div>
                <div className="flex items-center gap-3">
                    {currentTheme && (
                        <div className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-surface/50">
                            <div
                                className="w-4 h-4 rounded-full"
                                style={{ backgroundColor: currentTheme.palette.accent }}
                            />
                            <span className="text-xs text-textSecondary font-medium">{currentTheme.name}</span>
                        </div>
                    )}
                    <button
                        onClick={() => setIsCreating(true)}
                        className="flex items-center gap-2 px-3 py-1.5 glass-button text-sm font-medium"
                    >
                        <Plus size={14} />
                        Create Theme
                    </button>
                </div>
            </div>

            {/* Global Glassiness — scales every glass surface across every theme, from the
                signature frosted look (100%) down to a fully flat, solid, blur-free UI (0%). */}
            <div className="settings-card p-4 space-y-3">
                <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                        <Droplets size={16} className="text-accent" />
                        <h4 className="text-sm font-semibold text-textPrimary">Glassiness</h4>
                    </div>
                    <span className="text-xs text-textMuted font-mono">{liveGlass}%</span>
                </div>
                <input
                    type="range"
                    min="0"
                    max="100"
                    step="1"
                    value={liveGlass}
                    onChange={(e) => handleGlassInput(parseInt(e.target.value, 10))}
                    onPointerUp={(e) => commitGlass(parseInt(e.currentTarget.value, 10))}
                    onKeyUp={(e) => commitGlass(parseInt(e.currentTarget.value, 10))}
                    className="w-full accent-accent cursor-pointer"
                />
                <div className="flex items-center justify-between text-[10px] text-textMuted">
                    <span>Solid</span>
                    <span>Frosted glass</span>
                </div>
                <p className="text-xs text-textMuted">
                    How see-through and frosted every surface is, for every theme. 100% is the signature glass; 0% removes all transparency and blur for a completely flat, solid look.
                </p>
            </div>

            {/* Interface Font — palette-independent, like glassiness. Compact
                tiles: each font's NAME is drawn in that font, so the label is its
                own preview. Description is on hover (Tooltip) to keep this small. */}
            <div id="settings-section-font" className="settings-card p-4 space-y-3">
                <div className="flex items-center gap-2">
                    <Type size={16} className="text-accent" />
                    <h4 className="text-sm font-semibold text-textPrimary">Font</h4>
                </div>
                <div className="grid grid-cols-2 gap-2">
                    {FONT_OPTIONS.map((font) => {
                        const isSelected = currentFontId === font.id;
                        return (
                            <Tooltip key={font.id} content={font.description} side="top">
                                <button
                                    onClick={() => handleFontChange(font.id)}
                                    // translateZ(0) lifts the tile onto its own compositor
                                    // layer so the dialog's heavy liquid-glass backdrop-filter
                                    // (blur 96px) + scale-in don't rasterize/soften the in-font
                                    // name. Same crispness workaround the profile avatar uses.
                                    style={{ transform: 'translateZ(0)' }}
                                    className={`flex items-center justify-between gap-2 w-full px-3 py-2 rounded-lg border transition-colors duration-200 text-left ${
                                        isSelected
                                            ? 'border-accent bg-accent/10'
                                            : 'border-borderSubtle hover:border-borderLight'
                                    }`}
                                >
                                    {/* Name rendered in its own face at weight 400 (each
                                        font's true regular) — the label is the preview. */}
                                    <span
                                        className="text-[15px] leading-tight text-textPrimary truncate"
                                        style={{ fontFamily: font.stack, fontWeight: 400 }}
                                    >
                                        {font.label}
                                    </span>
                                    {isSelected && (
                                        <Check size={14} className="text-accent flex-shrink-0" strokeWidth={3} />
                                    )}
                                </button>
                            </Tooltip>
                        );
                    })}
                    {/* Custom — same tile language as the presets: once a name
                        is typed, the tile label IS the preview, in that face. */}
                    <Tooltip content="Type any font name. Free fonts load automatically, and fonts installed on this PC work too." side="top">
                        <button
                            onClick={() => handleFontChange(CUSTOM_FONT_ID)}
                            style={{ transform: 'translateZ(0)' }}
                            className={`flex items-center justify-between gap-2 w-full px-3 py-2 rounded-lg border transition-colors duration-200 text-left ${
                                isCustomFont
                                    ? 'border-accent bg-accent/10'
                                    : 'border-borderSubtle hover:border-borderLight'
                            }`}
                        >
                            <span
                                className="text-[15px] leading-tight text-textPrimary truncate"
                                style={{
                                    fontFamily: isCustomFont && customFontDraft ? customFontStack(customFontDraft) : undefined,
                                    fontWeight: 400,
                                }}
                            >
                                {isCustomFont && customFontDraft ? customFontDraft : 'Custom'}
                            </span>
                            {isCustomFont && (
                                <Check size={14} className="text-accent flex-shrink-0" strokeWidth={3} />
                            )}
                        </button>
                    </Tooltip>
                </div>
                {isCustomFont && (
                    <div className="space-y-2.5 pt-0.5">
                        <div>
                            <label className="text-xs text-textMuted mb-1 block">Font name</label>
                            <input
                                value={customFontDraft}
                                onChange={(e) => handleCustomFontChange(e.target.value)}
                                placeholder="e.g. Poppins"
                                aria-label="Custom font name"
                                spellCheck={false}
                                autoComplete="off"
                                style={{ fontFamily: customFontDraft ? customFontStack(customFontDraft) : undefined }}
                                className="w-full min-w-0 rounded-lg bg-glass border border-borderSubtle px-3 py-2 text-sm text-textPrimary placeholder:text-textMuted focus:outline-none focus:border-accent/60 transition-colors duration-200"
                            />
                        </div>
                        {/* Quick fills. Borderless so they read as a list of
                            names, not a row of buttons; each is drawn in its
                            own face (all eight preload in one request). */}
                        <div className="grid grid-cols-2 gap-x-2 gap-y-0.5">
                            {SUGGESTED_FONTS.map((name) => (
                                <button
                                    key={name}
                                    onClick={() => handleCustomFontChange(name)}
                                    style={{ fontFamily: customFontStack(name), transform: 'translateZ(0)' }}
                                    className={`px-2 py-1 rounded-md text-[13px] leading-tight text-left truncate transition-colors duration-200 hover:bg-white/5 ${
                                        customFontDraft === name ? 'text-accent' : 'text-textSecondary'
                                    }`}
                                >
                                    {name}
                                </button>
                            ))}
                        </div>
                        <p className="text-xs text-textMuted leading-relaxed">
                            Any free font from fonts.google.com works, just type its exact name. Fonts installed on this PC work too, with no download.
                        </p>
                    </div>
                )}
            </div>

            {/* Custom Themes Section */}
            {customThemes.length > 0 && (
                <div className="space-y-3">
                    <div className="flex items-center gap-2">
                        <PaintBucket size={16} className="text-accent" />
                        <h4 className="text-sm font-semibold text-textSecondary">Your Themes</h4>
                        <span className="text-xs text-textMuted">Your own creations</span>
                    </div>

                    <div className="grid grid-cols-3 gap-3">
                        {customThemes.map((customTheme) => {
                            const runtimeTheme = customThemeToTheme(customTheme);
                            return (
                                <ThemeCard
                                    key={customTheme.id}
                                    theme={runtimeTheme}
                                    isSelected={currentThemeId === customTheme.id}
                                    onSelect={() => handleThemeChange(customTheme.id)}
                                    isCustom={true}
                                    onEdit={() => handleEditTheme(customTheme)}
                                />
                            );
                        })}
                    </div>
                </div>
            )}

            {/* Theme Categories */}
            {themeCategories.map((category) => {
                const categoryThemes = themes.filter((t) => t.category === category.id);
                if (categoryThemes.length === 0) return null;

                return (
                    <div key={category.id} className="space-y-3">
                        <div className="flex items-center gap-2">
                            <span className="text-textMuted">{getCategoryIcon(category.id)}</span>
                            <h4 className="text-sm font-semibold text-textSecondary">{category.name}</h4>
                            <span className="text-xs text-textMuted">{category.description}</span>
                        </div>

                        <div className="grid grid-cols-3 gap-3">
                            {categoryThemes.map((theme) => {
                                // Preview the OLED card in its chosen accent.
                                const display = theme.id === OLED_THEME_ID ? getOledTheme(liveOledAccent) : theme;
                                return (
                                    <ThemeCard
                                        key={theme.id}
                                        theme={display}
                                        isSelected={currentThemeId === theme.id}
                                        onSelect={() => handleThemeChange(theme.id)}
                                    />
                                );
                            })}
                        </div>

                        {/* OLED accent chooser — shown only while OLED is the
                            active theme. Preset swatches for one-click colors,
                            plus a full spectrum picker for any color. */}
                        {category.id === 'signature' && currentThemeId === OLED_THEME_ID && (
                            <div className="settings-card p-4 space-y-3">
                                <div className="flex items-center gap-2">
                                    <Droplets size={16} className="text-accent" />
                                    <h4 className="text-sm font-semibold text-textPrimary">OLED accent</h4>
                                    <span className="text-xs text-textMuted">The glow color on pure black</span>
                                </div>
                                <div className="flex flex-wrap gap-2">
                                    {OLED_ACCENT_PRESETS.map((preset) => {
                                        const isSel = liveOledAccent.toLowerCase() === preset.value.toLowerCase();
                                        return (
                                            <Tooltip key={preset.value} content={preset.name} side="top">
                                                <button
                                                    onClick={() => handleOledAccentChange(preset.value)}
                                                    className={`w-7 h-7 rounded-full border transition-transform hover:scale-110 ${
                                                        isSel ? 'border-textPrimary' : 'border-borderSubtle'
                                                    }`}
                                                    style={{ backgroundColor: preset.value }}
                                                />
                                            </Tooltip>
                                        );
                                    })}
                                </div>
                                <ThemeColorPicker
                                    label="Custom color"
                                    color={{ value: liveOledAccent, opacity: 100 }}
                                    showOpacity={false}
                                    onChange={(c) => handleOledAccentChange(c.value)}
                                />
                            </div>
                        )}
                    </div>
                );
            })}

            {/* Theme Tips */}
            <div className="p-3 rounded-lg bg-surface/50 border border-borderSubtle mb-4">
                <p className="text-xs text-textMuted flex items-center gap-2">
                    <Sparkles size={14} />
                    Create a custom theme to match your setup exactly.
                </p>
            </div>
        </div>
    );
};

export default ThemeSettings;
