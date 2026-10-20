// Settings > Chat > Spam & Pyramid: how fast /spam and /pyramid send, and
// whether a repeated line is varied so Twitch keeps it. Rust reads both
// (models/settings.rs ChatBurstSettings) when a run starts.

import { useAppStore } from '../../stores/AppStore';
import type { ChatBurstSettings as ChatBurstPrefs } from '../../types';
import { SettingsSection, SettingsRow } from './_primitives';
import { useSettingReset } from './settingReset';
import { InlineSlider } from '../plugins/settingsPageKit';
import { Toggle } from '../ui/Toggle';
import { IS_MOBILE } from '../../utils/platform';

const DEFAULT_DELAY_MS = 1000;

const formatDelay = (ms: number) => (ms >= 1000 ? `${(ms / 1000).toFixed(ms % 1000 === 0 ? 0 : 2)} s` : `${ms} ms`);

const ChatBurstSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const resetFor = useSettingReset();
  const prefs = settings.chat_burst;
  const delay = prefs?.delay_ms ?? DEFAULT_DELAY_MS;
  const vary = prefs?.vary_repeats ?? true;

  const set = (patch: Partial<ChatBurstPrefs>) =>
    updateSettings({ ...settings, chat_burst: { ...settings.chat_burst, ...patch } });

  return (
    <SettingsSection
      id="settings-section-spam-pyramid"
      label="Spam & Pyramid"
      description="How /spam and /pyramid send their lines."
    >
      <SettingsRow
        title="Speed"
        onReset={resetFor(['chat_burst.delay_ms', DEFAULT_DELAY_MS])}
        description={`One line every ${formatDelay(delay)}. Add --delay <ms> to a command to change it for one run.`}
        help="1 second is the pace Twitch allows a regular viewer in one channel. Moderators, VIPs and the broadcaster may go faster. Going faster than your role allows gets lines dropped and can silence you for an hour, and moderators may time you out for spam. A run stops by itself if Twitch drops three lines in a row or says you're timed out."
        control={
          IS_MOBILE ? undefined : (
            <InlineSlider
              value={delay}
              min={10}
              max={5000}
              step={10}
              label="Milliseconds between lines"
              format={formatDelay}
              onChange={(delay_ms) => set({ delay_ms })}
            />
          )
        }
      >
        {/* The phone runs the slider full width under the row, as the iPhone
            does; the description already says the pause it sets. */}
        {IS_MOBILE && (
          <input
            type="range"
            min={10}
            max={5000}
            step={10}
            value={delay}
            aria-label="Milliseconds between lines"
            onChange={(e) => set({ delay_ms: Number(e.target.value) })}
            className="w-full accent-accent cursor-pointer"
          />
        )}
      </SettingsRow>
      <SettingsRow
        title="Vary repeats"
        onReset={resetFor(['chat_burst.vary_repeats', true])}
        description="Adds invisible characters to a line that repeats the one before it, so Twitch doesn't drop it as a duplicate."
        help="Twitch drops a message identical to your previous one sent less than 30 seconds earlier. With this off, /spam sends your text exactly as typed and repeats may be dropped."
        control={<Toggle enabled={vary} onChange={() => set({ vary_repeats: !vary })} />}
      />
    </SettingsSection>
  );
};

export default ChatBurstSettings;
