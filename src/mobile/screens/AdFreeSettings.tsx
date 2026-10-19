// Ad-free playback settings.
//
// Its own panel rather than a row inside the shared Player panel: that panel is
// the desktop component, and the desktop app resolves playback through its
// plugin seam, so these two settings mean nothing there.
//
// The relay list is deliberately buried behind a disclosure. The bundled pool
// is what almost everyone should be on, and a hand-typed override that goes
// stale is the most likely way for someone to end up watching ads while the
// feature reports itself as on.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ChevronDown, MonitorPlay } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { SettingsRow, SettingsSection } from '../../components/settings/_primitives';
import { useSettingReset } from '../../components/settings/settingReset';
import type { VideoPlayerSettings } from '../../types';
import { Logger } from '../../utils/logger';
import { fieldsIn, setSettingsFields } from '../../utils/settingsFields';

// How long the relay box waits after the last keystroke before saving.
const RELAY_SAVE_MS = 500;

const Toggle: React.FC<{ on: boolean; onChange: () => void }> = ({ on, onChange }) => (
  <button
    onClick={onChange}
    role="switch"
    aria-checked={on}
    className={`shrink-0 w-11 h-6 rounded-full relative transition-colors ${
      on ? 'bg-accent' : 'bg-surface'
    }`}
  >
    <span
      className="absolute top-0.5 w-5 h-5 rounded-full bg-white transition-[left] duration-200 ease-out"
      style={{ left: on ? 22 : 2 }}
    />
  </button>
);

/** Plain-language reading of how the stream now playing is being served. */
function describeSource(
  mode: string | undefined,
  entitled: boolean,
  region: string | undefined,
): { label: string; detail: string } {
  switch (mode) {
    case 'turbo':
      return {
        label: 'Turbo',
        detail: 'Your Turbo subscription already serves this stream without ads, so no relay is used.',
      };
    case 'subscribed':
      return {
        label: 'Subscribed',
        detail:
          'Your subscription to this channel already serves it without ads, so no relay is used.',
      };
    case 'proxy':
      return {
        label: region ? `Relay (${region})` : 'Relay',
        detail: 'The playlist is coming through a public relay, with ad segments stripped out.',
      };
    case 'auth-only':
      return {
        label: 'Direct',
        detail:
          'No relay answered, so this is the stream Twitch serves you directly. Ads can appear.',
      };
    default:
      return entitled
        ? { label: 'Ad-free', detail: 'This stream is being served without ads.' }
        : { label: 'Not playing', detail: 'Start a stream to see how it is being served.' };
  }
}

const AdFreeSettings: React.FC = () => {
  const video: Partial<VideoPlayerSettings> = useAppStore((s) => s.settings.video_player) ?? {};
  const adSource = useAppStore((s) => s.adSource);
  const resetFor = useSettingReset();
  const [relaysOpen, setRelaysOpen] = useState(false);

  const on = video.ad_bypass_enabled !== false;
  const stored = video.ad_bypass_proxies ?? '';

  // One field per write. The whole video_player group used to be spread from
  // this screen's copy on every keystroke, so a write in flight could put back
  // a sibling another screen had just changed, and the box lost characters
  // while each save round-tripped.
  const patch = useCallback(async (changes: Partial<VideoPlayerSettings>) => {
    try {
      await setSettingsFields(fieldsIn('video_player', changes));
    } catch (err) {
      Logger.warn('[AdFreeSettings] save failed:', err);
      useAppStore.getState().addToast('Could not save that change.', 'error');
    }
  }, []);

  // The relay box types into its own draft and saves it after a pause, so the
  // store's echo of a save never overwrites what is being typed. The stored
  // value takes over again once it has caught up with the last save.
  const [draft, setDraft] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  if (draft !== null && !dirty && draft === stored) setDraft(null);
  const relays = draft ?? stored;
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingRelays = useRef<string | null>(null);
  const flushRelays = useCallback(() => {
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = null;
    const value = pendingRelays.current;
    pendingRelays.current = null;
    if (value === null) return;
    void patch({ ad_bypass_proxies: value }).finally(() => {
      if (pendingRelays.current === null) setDirty(false);
    });
  }, [patch]);
  const editRelays = (value: string) => {
    setDraft(value);
    setDirty(true);
    pendingRelays.current = value;
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(flushRelays, RELAY_SAVE_MS);
  };
  // Leaving the screen mid-edit still saves it.
  useEffect(() => () => flushRelays(), [flushRelays]);

  const source = useMemo(
    () => describeSource(adSource?.mode, !!adSource?.entitled, adSource?.region),
    [adSource],
  );

  // The arrow writes the whole settings like every other row's; a relay edit
  // still waiting to save is dropped first, so it cannot land after the reset.
  const restoreRelays = resetFor(['video_player.ad_bypass_proxies', '']);
  const resetRelays =
    restoreRelays &&
    (() => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = null;
      pendingRelays.current = null;
      setDraft(null);
      setDirty(false);
      restoreRelays();
    });

  // Sections in the settings pages' grouped style, named as the settings
  // catalog names them so a search hit lands on its row.
  return (
    <div className="space-y-6">
      <SettingsSection label="Ad-free">
        <SettingsRow
          title="Ad-free playback"
          description={
            on
              ? 'Routes the playlist through a public relay and removes ad segments before the player sees them.'
              : 'Streams play exactly as Twitch serves them.'
          }
          onReset={resetFor(['video_player.ad_bypass_enabled', true])}
          control={<Toggle on={on} onChange={() => void patch({ ad_bypass_enabled: !on })} />}
        />
      </SettingsSection>

      {on && (
        <>
          <SettingsSection label="This stream">
            {/* How it is served, not a setting: no control and no arrow. */}
            <SettingsRow icon={MonitorPlay} title={source.label} description={source.detail} />
          </SettingsSection>

          <SettingsSection label="Good to know">
            <div className="py-3 flex flex-col gap-2.5 text-[13px] leading-[18px] text-textSecondary">
              <p>
                A Turbo subscription or a sub to the channel is already ad-free through Twitch
                itself, and gives you the highest quality tiers. Those streams skip the relay.
              </p>
              <p>
                Low Latency stays off while this is on. Both want to build the playlist the player
                reads, and only one of them can.
              </p>
            </div>
          </SettingsSection>

          {/* Still behind a disclosure: the row says which list is in use, and
              the box to replace it only opens on purpose. */}
          <SettingsSection label="Relays">
            <SettingsRow
              title="Relays"
              description={relays.trim() ? 'Your own list' : 'The bundled list'}
              onReset={resetRelays}
              control={
                <button
                  type="button"
                  onClick={() => setRelaysOpen((v) => !v)}
                  aria-expanded={relaysOpen}
                  aria-label={relaysOpen ? 'Hide the relay list' : 'Show the relay list'}
                  // The tap target is a full 44px, pulled in so the line keeps
                  // its height and the chevron ends where a toggle does.
                  className="sn-touch -my-2.5 -mr-[14px] flex items-center justify-center text-textMuted"
                >
                  <ChevronDown size={16} className={`transition-transform ${relaysOpen ? 'rotate-180' : ''}`} />
                </button>
              }
            >
              {relaysOpen && (
                <div>
                  <p className="text-[13px] leading-[18px] text-textSecondary mb-2.5">
                    Leave this empty to use the bundled list, which is raced on every stream so the
                    fastest one that answers wins. Enter your own addresses, one per line, only if you
                    run or trust a specific relay.
                  </p>
                  <p className="text-[13px] leading-[18px] text-textSecondary mb-2.5">
                    Your own list is used on its own. If none of them answer, the stream plays
                    directly from Twitch with ads rather than falling back to the bundled relays.
                  </p>
                  <textarea
                    value={relays}
                    onChange={(e) => editRelays(e.target.value)}
                    onBlur={flushRelays}
                    rows={3}
                    spellCheck={false}
                    autoCapitalize="none"
                    autoCorrect="off"
                    placeholder="https://relay.example.com"
                    className="glass-input w-full rounded-lg px-3 py-2.5 text-[13px] font-mono text-textPrimary placeholder:text-textMuted outline-none resize-none"
                  />
                </div>
              )}
            </SettingsRow>
          </SettingsSection>
        </>
      )}
    </div>
  );
};

export default AdFreeSettings;
