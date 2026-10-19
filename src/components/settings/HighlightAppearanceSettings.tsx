import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { useClientCapabilities } from '../../stores/clientCapabilitiesStore';
import { Logger } from '../../utils/logger';
// Rendered inside ChatSettings, so this reaches the phone too. Shared panel,
// so a platform branch here is legitimate.
import { IS_MOBILE } from '../../utils/platform';
import { SettingsSection, SettingsRow, SegmentedSelect } from './_primitives';
import { InlineSlider, SubControl, SubControls } from '../plugins/settingsPageKit';
import type { ChatDesignSettings, HighlightDisplayStyle } from '../../types';
import { useSettingReset } from './settingReset';
import { ScrollbarMarksPreview, type PreviewDesign } from './ChatPreview';
import { FALLBACK_MARK_KINDS, MARK_KIND_OPTIONS } from '../chat/scrollbarMarkLayout';

const STYLE_HINTS: Record<HighlightDisplayStyle, string> = {
  standard: 'Tinted row background plus colored left border.',
  minimal: 'Colored left border only, no row tint.',
  none: 'Nothing drawn. The sound and title flash still play.',
};

// A phone has no window title to flash, so its None says only what the row shows.
const PHONE_STYLE_HINTS: Record<HighlightDisplayStyle, string> = {
  ...STYLE_HINTS,
  none: 'Nothing drawn on the row.',
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
  // Only a build that draws a chat scrollbar has marks to set (Rust client capabilities).
  const caps = useClientCapabilities();
  const appearance = settings.chat_highlights?.appearance ?? {};
  const displayStyle: HighlightDisplayStyle = appearance.display_style ?? 'standard';
  const opacity = appearance.opacity ?? 20;
  const resetFor = useSettingReset();

  const design = settings.chat_design;
  // Rust always sends a full chat_design block; the cast keeps the spread typed.
  const writeDesign = (patch: Partial<ChatDesignSettings>) =>
    updateSettings({ ...settings, chat_design: { ...(design as ChatDesignSettings), ...patch } });
  const marksOn = design?.scrollbar_marks ?? true;
  const markKinds = design?.scrollbar_mark_kinds ?? [...FALLBACK_MARK_KINDS];
  // Rust keeps the set in the settings order, so a list toggled back to the
  // default compares equal to it and the reset arrow goes away.
  const toggleMarkKind = (kind: string) => {
    void invoke('edit_settings_list', {
      path: 'chat_design.scrollbar_mark_kinds',
      edit: { op: 'toggle', value: kind },
    }).catch((err) => Logger.warn('[HighlightAppearance] mark kind toggle failed:', err));
  };
  const previewDesign: PreviewDesign = {
    show_dividers: design?.show_dividers ?? true,
    alternating_backgrounds: design?.alternating_backgrounds ?? false,
    message_spacing: design?.message_spacing ?? 8,
    font_size: design?.font_size ?? 14,
    font_weight: design?.font_weight ?? 400,
    show_timestamps: design?.show_timestamps ?? false,
    show_timestamp_seconds: design?.show_timestamp_seconds ?? false,
    timestamp_format: design?.timestamp_format ?? '12h',
    username_separator: design?.username_separator ?? (design?.username_colon ? 'colon' : 'none'),
    username_style: design?.username_style ?? 'plain',
    username_accent_source: design?.username_accent_source ?? 'user',
  };

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
        description={(IS_MOBILE ? PHONE_STYLE_HINTS : STYLE_HINTS)[displayStyle]}
        help="Opacity is how bright the row tint appears, in the Standard style only. 20% matches the original look."
      >
        <SegmentedSelect<HighlightDisplayStyle>
          fullWidth={IS_MOBILE}
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

      {caps.scrollbar_marks && (
        <SettingsRow
          title="Scrollbar marks"
          onReset={resetFor('chat_design.scrollbar_marks')}
          description="Small ticks on the chat scrollbar show where mentions, subs, announcements and pinned messages are, so you can find what you missed while reading back."
          help="A tick takes its message's highlight color where it has one. A run of one kind, like a gift bomb, joins into one bar, and mentions draw on top of everything else."
          control={
            <Toggle
              enabled={marksOn}
              onChange={() => writeDesign({ scrollbar_marks: !marksOn })}
            />
          }
        >
          {marksOn && (
            <div className="space-y-4">
              <SubControls>
                <SubControl
                  title="Kinds"
                  stacked
                  onReset={resetFor('chat_design.scrollbar_mark_kinds')}
                  control={
                    <div className="flex flex-wrap items-center gap-1.5">
                      {MARK_KIND_OPTIONS.map(({ kind, label }) => {
                        const on = markKinds.includes(kind);
                        return (
                          <button
                            key={kind}
                            type="button"
                            aria-pressed={on}
                            onClick={() => toggleMarkKind(kind)}
                            className={`px-2.5 py-1 rounded-full text-[12px] font-medium transition-colors ${
                              on
                                ? 'chrome-glaze chrome-glaze--flat chrome-glaze--control text-textPrimary'
                                : 'glass-button-static text-textMuted'
                            }`}
                          >
                            {label}
                          </button>
                        );
                      })}
                    </div>
                  }
                />
              </SubControls>
              <ScrollbarMarksPreview design={previewDesign} kinds={markKinds} mentionColor={design?.mention_color} />
            </div>
          )}
        </SettingsRow>
      )}
    </SettingsSection>
  );
};

export default HighlightAppearanceSettings;
