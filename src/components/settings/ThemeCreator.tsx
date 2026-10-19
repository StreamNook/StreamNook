import { useState, useEffect } from 'react';
import { Dropdown } from '../ui/Dropdown';
import { ChevronDown, ChevronRight, Palette, Trash2, Save, X, Wand2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import ThemeColorPicker from '../ThemeColorPicker';
import { Tooltip } from '../ui/Tooltip';
import {
  themes,
  getThemeById,
  applyTheme,
  createDefaultCustomPalette,
  customThemeToTheme,
  OLED_THEME_ID,
  getOledTheme,
} from '../../themes';
import type { CustomTheme, CustomThemePalette, CustomThemeColor } from '../../types';

interface ThemeCreatorProps {
  editingTheme?: CustomTheme;
  onClose: () => void;
  onSave: (theme: CustomTheme) => void;
  onDelete?: (themeId: string) => void;
}

interface ColorSection {
  id: string;
  name: string;
  colors: Array<{
    key: keyof CustomThemePalette | string;
    label: string;
    path?: string[]; // For nested properties like highlight.pink
  }>;
}

const COLOR_SECTIONS: ColorSection[] = [
  {
    id: 'core',
    name: 'Core Colors',
    colors: [
      { key: 'background', label: 'Background' },
      { key: 'backgroundSecondary', label: 'Background Secondary' },
      { key: 'backgroundTertiary', label: 'Background Tertiary' },
    ],
  },
  {
    id: 'surface',
    name: 'Surface Colors',
    colors: [
      { key: 'surface', label: 'Surface' },
      { key: 'surfaceHover', label: 'Surface Hover' },
      { key: 'surfaceActive', label: 'Surface Active' },
    ],
  },
  {
    id: 'text',
    name: 'Text Colors',
    colors: [
      { key: 'textPrimary', label: 'Text Primary' },
      { key: 'textSecondary', label: 'Text Secondary' },
      { key: 'textMuted', label: 'Text Muted' },
    ],
  },
  {
    id: 'accent',
    name: 'Accent Colors',
    colors: [
      { key: 'accent', label: 'Accent' },
      { key: 'accentHover', label: 'Accent Hover' },
      { key: 'accentMuted', label: 'Accent Muted' },
    ],
  },
  {
    id: 'border',
    name: 'Border Colors',
    colors: [
      { key: 'border', label: 'Border' },
      { key: 'borderLight', label: 'Border Light' },
      { key: 'borderSubtle', label: 'Border Subtle' },
    ],
  },
  {
    id: 'semantic',
    name: 'Semantic Colors',
    colors: [
      { key: 'success', label: 'Success' },
      { key: 'warning', label: 'Warning' },
      { key: 'error', label: 'Error' },
      { key: 'info', label: 'Info' },
    ],
  },
  {
    id: 'scrollbar',
    name: 'Scrollbar',
    colors: [
      { key: 'scrollbarThumb', label: 'Scrollbar Thumb' },
      { key: 'scrollbarTrack', label: 'Scrollbar Track' },
    ],
  },
  {
    id: 'highlight',
    name: 'Highlight Colors',
    colors: [
      { key: 'highlight.pink', label: 'Pink', path: ['highlight', 'pink'] },
      { key: 'highlight.purple', label: 'Purple', path: ['highlight', 'purple'] },
      { key: 'highlight.blue', label: 'Blue', path: ['highlight', 'blue'] },
      { key: 'highlight.cyan', label: 'Cyan', path: ['highlight', 'cyan'] },
      { key: 'highlight.green', label: 'Green', path: ['highlight', 'green'] },
      { key: 'highlight.yellow', label: 'Yellow', path: ['highlight', 'yellow'] },
      { key: 'highlight.orange', label: 'Orange', path: ['highlight', 'orange'] },
      { key: 'highlight.red', label: 'Red', path: ['highlight', 'red'] },
    ],
  },
];

const ThemeCreator = ({ editingTheme, onClose, onSave, onDelete }: ThemeCreatorProps) => {
  const { settings } = useAppStore();
  
  // Initialize palette
  const getInitialPalette = (): CustomThemePalette => {
    if (editingTheme) {
      return editingTheme.palette;
    }
    const baseTheme = getThemeById('winters-glass') || themes[0];
    return createDefaultCustomPalette(baseTheme);
  };

  const [themeName, setThemeName] = useState(editingTheme?.name || '');
  const [baseThemeId, setBaseThemeId] = useState('winters-glass');
  const [palette, setPalette] = useState<CustomThemePalette>(getInitialPalette);
  const [expandedSections, setExpandedSections] = useState<Set<string>>(new Set(['core', 'accent']));
  const [hasChanges, setHasChanges] = useState(false);

  // Apply live preview when palette changes. It paints at once; applyTheme then
  // takes Rust's floors (muted and secondary text, the accent on cards), the
  // same ones saving applies, so the preview is the theme as it will be saved.
  useEffect(() => {
    const previewTheme: CustomTheme = {
      id: 'preview',
      name: 'Preview',
      createdAt: Date.now(),
      updatedAt: Date.now(),
      palette,
    };
    applyTheme(customThemeToTheme(previewTheme));
    queueMicrotask(() => setHasChanges(true));
  }, [palette]);

  // Restore original theme on unmount or cancel
  const restoreOriginalTheme = () => {
    const originalId = settings.theme || 'winters-glass';
    const customThemes = settings.custom_themes || [];
    const customTheme = customThemes.find((t) => t.id === originalId);
    
    if (customTheme) {
      applyTheme(customThemeToTheme(customTheme));
    } else if (originalId === OLED_THEME_ID) {
      // OLED's accent is user-chosen; restore it with the saved accent, not the
      // static default, so canceling the creator doesn't drop the chosen color.
      applyTheme(getOledTheme(settings.oled_accent));
    } else {
      const builtInTheme = getThemeById(originalId) || themes[0];
      applyTheme(builtInTheme);
    }
  };

  const handleCancel = () => {
    restoreOriginalTheme();
    onClose();
  };

  const handleBaseThemeChange = (themeId: string) => {
    setBaseThemeId(themeId);
    const baseTheme = getThemeById(themeId);
    if (baseTheme) {
      setPalette(createDefaultCustomPalette(baseTheme));
    }
  };

  const toggleSection = (sectionId: string) => {
    setExpandedSections((prev) => {
      const next = new Set(prev);
      if (next.has(sectionId)) {
        next.delete(sectionId);
      } else {
        next.add(sectionId);
      }
      return next;
    });
  };

  const getColorValue = (key: string, path?: string[]): CustomThemeColor => {
    if (path && path.length === 2) {
      const parent = palette[path[0] as keyof CustomThemePalette];
      if (typeof parent === 'object' && parent !== null && 'pink' in parent) {
        return parent[path[1] as keyof typeof parent] as CustomThemeColor;
      }
    }
    return palette[key as keyof CustomThemePalette] as CustomThemeColor;
  };

  const setColorValue = (key: string, value: CustomThemeColor, path?: string[]) => {
    setPalette((prev) => {
      if (path && path.length === 2) {
        const parentKey = path[0] as 'highlight';
        const childKey = path[1] as keyof typeof prev.highlight;
        return {
          ...prev,
          [parentKey]: {
            ...prev[parentKey],
            [childKey]: value,
          },
        };
      }
      return {
        ...prev,
        [key]: value,
      };
    });
  };

  const handleSave = () => {
    if (!themeName.trim()) {
      return; // Could add validation feedback
    }

    const now = Date.now();
    const theme: CustomTheme = {
      id: editingTheme?.id || `custom-${now}`,
      name: themeName.trim(),
      createdAt: editingTheme?.createdAt || now,
      updatedAt: now,
      palette,
    };

    onSave(theme);
  };

  const handleGlassOpacityChange = (key: 'glassOpacity' | 'glassHoverOpacity' | 'glassActiveOpacity', value: string) => {
    setPalette((prev) => ({
      ...prev,
      [key]: value,
    }));
  };

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div className="p-2 rounded-lg bg-accent/20">
            <Palette size={20} className="text-accent" />
          </div>
          <div>
            <h3 className="text-lg font-semibold text-textPrimary">
              {editingTheme ? 'Edit Theme' : 'Create Custom Theme'}
            </h3>
            <p className="text-xs text-textMuted">
              Customize every color to create your perfect aesthetic
            </p>
          </div>
        </div>
        <button
          onClick={handleCancel}
          className="p-2 text-textSecondary hover:text-textPrimary hover:bg-glass rounded transition-all"
        >
          <X size={20} />
        </button>
      </div>

      {/* Theme Name Input */}
      <div>
        <label className="block text-sm font-medium text-textPrimary mb-2">
          Theme Name
        </label>
        <input
          type="text"
          value={themeName}
          onChange={(e) => setThemeName(e.target.value)}
          placeholder="My Custom Theme"
          className="w-full px-4 py-2.5 glass-input text-textPrimary text-sm"
        />
      </div>

      {/* Base Theme Selector (only for new themes) */}
      {!editingTheme && (
        <div>
          <label className="block text-sm font-medium text-textPrimary mb-2">
            Start from Base Theme
          </label>
          <div className="flex items-center gap-3">
            <Dropdown
              value={baseThemeId}
              onChange={handleBaseThemeChange}
              className="flex-1"
              ariaLabel="Base theme"
              options={themes.map((theme) => ({ value: theme.id, label: theme.name }))}
            />
            <Tooltip content="Reset to base theme" side="top">
            <button
              onClick={() => handleBaseThemeChange(baseThemeId)}
              className="px-4 py-2.5 glass-button text-textSecondary hover:text-textPrimary flex items-center gap-2"
            >
              <Wand2 size={16} />
              Reset
            </button>
            </Tooltip>
          </div>
        </div>
      )}

      {/* Color Sections */}
      <div className="space-y-2">
        {COLOR_SECTIONS.map((section) => (
          <div key={section.id} className="glass-panel rounded-lg overflow-hidden">
            <button
              onClick={() => toggleSection(section.id)}
              className="w-full flex items-center justify-between p-3 hover:bg-glass-hover transition-colors"
            >
              <span className="text-sm font-medium text-textPrimary">{section.name}</span>
              {expandedSections.has(section.id) ? (
                <ChevronDown size={16} className="text-textMuted" />
              ) : (
                <ChevronRight size={16} className="text-textMuted" />
              )}
            </button>

            {expandedSections.has(section.id) && (
              <div className="p-3 pt-0 space-y-3 border-t border-borderSubtle">
                {section.colors.map((colorDef) => (
                  <ThemeColorPicker
                    key={colorDef.key}
                    label={colorDef.label}
                    color={getColorValue(colorDef.key, colorDef.path)}
                    onChange={(value) => setColorValue(colorDef.key, value, colorDef.path)}
                    showOpacity={true}
                  />
                ))}
              </div>
            )}
          </div>
        ))}

        {/* Glass Opacities Section */}
        <div className="glass-panel rounded-lg overflow-hidden">
          <button
            onClick={() => toggleSection('glass')}
            className="w-full flex items-center justify-between p-3 hover:bg-glass-hover transition-colors"
          >
            <span className="text-sm font-medium text-textPrimary">Glass Effect</span>
            {expandedSections.has('glass') ? (
              <ChevronDown size={16} className="text-textMuted" />
            ) : (
              <ChevronRight size={16} className="text-textMuted" />
            )}
          </button>

          {expandedSections.has('glass') && (
            <div className="p-3 pt-0 space-y-4 border-t border-borderSubtle">
              <div>
                <label className="text-sm text-textSecondary mb-2 block">
                  Glass Opacity: {parseFloat(palette.glassOpacity) * 100}%
                </label>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.01"
                  value={palette.glassOpacity}
                  onChange={(e) => handleGlassOpacityChange('glassOpacity', e.target.value)}
                  className="w-full accent-accent cursor-pointer"
                />
              </div>
              <div>
                <label className="text-sm text-textSecondary mb-2 block">
                  Glass Hover Opacity: {parseFloat(palette.glassHoverOpacity) * 100}%
                </label>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.01"
                  value={palette.glassHoverOpacity}
                  onChange={(e) => handleGlassOpacityChange('glassHoverOpacity', e.target.value)}
                  className="w-full accent-accent cursor-pointer"
                />
              </div>
              <div>
                <label className="text-sm text-textSecondary mb-2 block">
                  Glass Active Opacity: {parseFloat(palette.glassActiveOpacity) * 100}%
                </label>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.01"
                  value={palette.glassActiveOpacity}
                  onChange={(e) => handleGlassOpacityChange('glassActiveOpacity', e.target.value)}
                  className="w-full accent-accent cursor-pointer"
                />
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Action Buttons */}
      <div className="flex items-center justify-between pt-4 border-t border-borderSubtle">
        {editingTheme && onDelete ? (
          <button
            onClick={() => onDelete(editingTheme.id)}
            className="px-4 py-2 text-sm text-error hover:bg-error/10 rounded-lg transition-colors flex items-center gap-2"
          >
            <Trash2 size={16} />
            Delete Theme
          </button>
        ) : (
          <div />
        )}

        <div className="flex items-center gap-3">
          <button
            onClick={handleCancel}
            className="px-4 py-2 text-sm text-textSecondary hover:text-textPrimary hover:bg-glass rounded-lg transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={!themeName.trim()}
            className="px-4 py-2 glass-button text-sm font-medium flex items-center gap-2 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <Save size={16} />
            {editingTheme ? 'Update Theme' : 'Save Theme'}
          </button>
        </div>
      </div>

      {/* Live Preview Note */}
      {hasChanges && (
        <p className="text-xs text-textMuted text-center">
          ✨ Changes are previewed live. Save to keep your theme.
        </p>
      )}
    </div>
  );
};

export default ThemeCreator;
