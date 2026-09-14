import { useAppStore } from '../../stores/AppStore';
// Rendered inside ChatSettings, so this reaches the phone too. Shared panel,
// so a platform branch here is legitimate.
import { IS_MOBILE } from '../../utils/platform';
import { SettingsSection, SettingsRow, SegmentedSelect } from './_primitives';
import type { HighlightDisplayStyle } from '../../types';

const STYLE_HINTS: Record<HighlightDisplayStyle, string> = {
  standard: 'Tinted row background plus colored left border.',
  minimal: 'Colored left border only — no row tint.',
  none: 'No visual. Sound and title-flash still fire.',
};

const Toggle = ({ enabled, onChange }: { enabled: boolean; onChange: () => void }) => (
  <button
    onClick={onChange}
    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors flex-shrink-0 ${
      enabled ? 'bg-accent' : 'bg-gray-600'
    }`}
  >
    <span
      className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
        enabled ? 'translate-x-6' : 'translate-x-1'
      }`}
    />
  </button>
);

const HighlightAppearanceSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const appearance = settings.chat_highlights?.appearance ?? {};
  const displayStyle: HighlightDisplayStyle = appearance.display_style ?? 'standard';
  const opacity = appearance.opacity ?? 20;

  const writeAppearance = (patch: Partial<typeof appearance>) =>
    updateSettings({
      ...settings,
      chat_highlights: {
        phrases: settings.chat_highlights?.phrases ?? [],
        ...settings.chat_highlights,
        appearance: { ...appearance, ...patch },
      },
    });

  return (
    <SettingsSection
      label="Highlight Appearance"
      description="Applies to every highlight type below — phrases, usernames, badges, and built-in events."
    >
      <SettingsRow
        title="Display style"
        description={STYLE_HINTS[displayStyle]}
      >
        <SegmentedSelect<HighlightDisplayStyle>
          value={displayStyle}
          onChange={(value) => writeAppearance({ display_style: value })}
          options={[
            { value: 'standard', label: 'Standard' },
            { value: 'minimal', label: 'Minimal' },
            { value: 'none', label: 'None' },
          ]}
        />
      </SettingsRow>

      <SettingsRow
        title={`Tint opacity: ${opacity}%`}
        description={
          displayStyle === 'standard'
            ? 'How bright the row tint appears. 20% matches the previous default.'
            : 'Disabled in this display style — only the standard variant uses the row tint.'
        }
        disabled={displayStyle !== 'standard'}
      >
        <input
          type="range"
          min="0"
          max="100"
          step="5"
          value={opacity}
          onChange={(e) => writeAppearance({ opacity: parseInt(e.target.value, 10) })}
          className="w-full accent-accent cursor-pointer"
        />
      </SettingsRow>

      {/* Desktop only: there is no window title to flash on Android and nothing
          to tab back from. The phone surfaces background highlights through
          system notifications instead (Settings > Notifications). */}
      {!IS_MOBILE && (
        <SettingsRow
          title="Flash window title when unfocused"
          description="When a highlight lands while StreamNook is in the background, flash the window title until you tab back. Messages older than 5 seconds (history backfill) are skipped."
          control={
            <Toggle
              enabled={appearance.flash_title_when_unfocused ?? false}
              onChange={() =>
                writeAppearance({ flash_title_when_unfocused: !appearance.flash_title_when_unfocused })
              }
            />
          }
        />
      )}
    </SettingsSection>
  );
};

export default HighlightAppearanceSettings;
