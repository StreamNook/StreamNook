import React from 'react';
import { useAppStore } from '../../stores/AppStore';
import { SettingsSection, SettingsRow, SegmentedSelect } from './_primitives';
import { Dropdown } from '../ui/Dropdown';
import type { BuiltInHighlightSettings, BuiltInHighlightRule } from '../../types';

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

type RuleKey = keyof BuiltInHighlightSettings;

const ROWS: Array<{
  key: RuleKey;
  label: string;
  hint: string;
  defaultColor: string;
  defaultEnabled: boolean;
}> = [
  {
    key: 'first_time_chatter',
    label: 'First-time chatters',
    hint: "Highlight a user's first ever message in this channel (Twitch first-msg tag). Already on by default with the purple gradient — toggle off or re-color from here.",
    defaultColor: '#a855f7',
    defaultEnabled: true,
  },
  {
    key: 'returning_chatter',
    label: 'Returning chatters',
    hint: 'Highlight users coming back after a long absence (Twitch returning-chatter tag).',
    defaultColor: '#22d3ee',
    defaultEnabled: false,
  },
  {
    key: 'self_message',
    label: 'Your own messages',
    hint: "Tint your own outgoing messages so they're easy to spot in fast chats.",
    defaultColor: '#facc15',
    defaultEnabled: false,
  },
  {
    key: 'raider',
    label: 'Raid announcements',
    hint: 'Highlight the "X is raiding with Y viewers" notice row when a raid lands.',
    defaultColor: '#ef4444',
    defaultEnabled: false,
  },
];

const BuiltInHighlightsSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const builtIn = settings.chat_highlights?.built_in ?? {};

  const patchRule = (key: RuleKey, patch: Partial<BuiltInHighlightRule>) => {
    const row = ROWS.find((r) => r.key === key)!;
    const current: BuiltInHighlightRule = builtIn[key] ?? {
      enabled: row.defaultEnabled,
      color: row.defaultColor,
    };
    updateSettings({
      ...settings,
      chat_highlights: {
        phrases: settings.chat_highlights?.phrases ?? [],
        ...settings.chat_highlights,
        built_in: {
          ...builtIn,
          [key]: { ...current, ...patch },
        },
      },
    });
  };

  return (
    <SettingsSection
      label="Built-in Event Highlights"
      description="Auto-highlight messages from specific event types. Runs alongside your custom phrase highlights and does not affect mention or reply flashes. Monitored / restricted suspicious-user highlights need the Twitch low-trust-users PubSub topic and are coming in a follow-up."
    >
      {ROWS.map((row) => {
        const rule: BuiltInHighlightRule = builtIn[row.key] ?? {
          enabled: row.defaultEnabled,
          color: row.defaultColor,
        };
        const firstTimeExtras = row.key === 'first_time_chatter' && rule.enabled;
        return (
          <React.Fragment key={row.key}>
            <SettingsRow
              title={row.label}
              description={row.hint}
              control={
                <div className="flex items-center gap-2">
                  <input
                    type="color"
                    value={rule.color}
                    onChange={(e) => patchRule(row.key, { color: e.target.value })}
                    className="w-8 h-8 rounded cursor-pointer bg-transparent border border-borderSubtle"
                    aria-label="Highlight color"
                  />
                  <Toggle enabled={rule.enabled} onChange={() => patchRule(row.key, { enabled: !rule.enabled })} />
                </div>
              }
            />
            {firstTimeExtras && (
              <>
                <SettingsRow
                  title="First-time look"
                  description="A tinted wash with a bar down the left, or a ring around the message."
                >
                  <SegmentedSelect<'wash' | 'ring'>
                    value={rule.style ?? 'wash'}
                    onChange={(style) => patchRule(row.key, { style })}
                    options={[
                      { value: 'wash', label: 'Wash' },
                      { value: 'ring', label: 'Ring' },
                    ]}
                  />
                </SettingsRow>
                {(rule.style ?? 'wash') === 'ring' && (
                  <SettingsRow
                    title="Fill inside the ring"
                    description="A faint color-matched fill, so the row reads as highlighted and not only outlined."
                    control={<Toggle enabled={rule.fill ?? false} onChange={() => patchRule(row.key, { fill: !(rule.fill ?? false) })} />}
                  />
                )}
                <SettingsRow
                  title="First-time glint"
                  description="A short highlight as the row lands: a sheen, a pulse, or a spark around the edge."
                >
                  <Dropdown<'none' | 'sheen' | 'pulse' | 'chase'>
                    value={rule.animation ?? 'none'}
                    onChange={(animation) => patchRule(row.key, { animation })}
                    className="w-full"
                    ariaLabel="First-time glint"
                    options={[
                      { value: 'none', label: 'None' },
                      { value: 'sheen', label: 'Sheen' },
                      { value: 'pulse', label: 'Pulse' },
                      { value: 'chase', label: 'Chase' },
                    ]}
                  />
                </SettingsRow>
                {(rule.animation ?? 'none') !== 'none' && (
                  <SettingsRow
                    title="Keep the glint going"
                    description="Off plays it once."
                    control={
                      <Toggle
                        enabled={rule.animate_repeat ?? false}
                        onChange={() => patchRule(row.key, { animate_repeat: !(rule.animate_repeat ?? false) })}
                      />
                    }
                  />
                )}
              </>
            )}
          </React.Fragment>
        );
      })}
    </SettingsSection>
  );
};

export default BuiltInHighlightsSettings;
