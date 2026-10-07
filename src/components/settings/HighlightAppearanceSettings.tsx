import { useAppStore } from '../../stores/AppStore';
// Rendered inside ChatSettings, so this reaches the phone too. Shared panel,
// so a platform branch here is legitimate.
import { IS_MOBILE } from '../../utils/platform';
import { SettingsSection, SettingsRow, SegmentedSelect } from './_primitives';
import { InlineSlider, SubControl, SubControls } from '../plugins/settingsPageKit';
import type { HighlightDisplayStyle } from '../../types';
import { useSettingReset } from './settingReset';

const STYLE_HINTS: Record<HighlightDisplayStyle, string> = {
  standard: 'Tinted row background plus colored left border.',
  minimal: 'Colored left border only, no row tint.',
  none: 'Nothing drawn. The sound and title flash still play.',
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
  const resetFor = useSettingReset();

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
      description="Applies to every highlight type below: phrases, usernames, badges, and events."
    >
      <SettingsRow
        title="Style"
        onReset={resetFor(['chat_highlights.appearance.display_style', 'standard'])}
        description={STYLE_HINTS[displayStyle]}
        help="Opacity is how bright the row tint appears, in the Standard style only. 20% matches the original look."
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
        {displayStyle === 'standard' && (
          <SubControls>
            <SubControl
              title="Opacity"
              onReset={resetFor(['chat_highlights.appearance.opacity', 20])}
              control={
                <InlineSlider
                  value={opacity}
                  min={0}
                  max={100}
                  step={5}
                  label="Highlight tint opacity"
                  format={(v) => `${v}%`}
                  onChange={(v) => writeAppearance({ opacity: v })}
                />
              }
            />
          </SubControls>
        )}
      </SettingsRow>

      {/* Desktop only: there is no window title to flash on Android and nothing
          to tab back from. The phone surfaces background highlights through
          system notifications instead (Settings > Notifications). */}
      {!IS_MOBILE && (
        <SettingsRow
          title="Title flash"
          onReset={resetFor(['chat_highlights.appearance.flash_title_when_unfocused', false])}
          description="Flashes the window title when a highlight lands while StreamNook is in the background, until you tab back."
          help="Chat history loaded when you join a channel never flashes."
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
