import { Dropdown } from '../ui/Dropdown';
import { useAppStore } from '../../stores/AppStore';
// Shared panel: both shells render it, so a platform branch here is legitimate.
// Hides rows whose backing feature does not exist on Android.
import { IS_MOBILE } from '../../utils/platform';
import { SettingsSection, SettingsRow, SegmentedSelect } from './_primitives';
import { DEFAULT_AUDIO_BOOST, DEFAULT_SONG_ID } from '../../types';
import { aboutRevealNeedsShift } from '../../utils/playerMouseControls';
import { Fader } from '../AudioBoostFaders';
import {
  audioBoostFaderDefs,
  audioBoostResetPatch,
  AUDIO_GRAPH_SUPPORTED,
  AUDIO_GRAPH_REFUSAL,
} from '../../utils/audioBoost';
import { reportCodecPreference } from '../../utils/codecPreference';
import { invoke } from '@tauri-apps/api/core';
import { AUTO_GAP } from '../../utils/latency';

const Toggle = ({ enabled, onChange }: { enabled: boolean; onChange: () => void }) => (
  <button
    onClick={onChange}
    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors flex-shrink-0 ${enabled ? 'bg-accent' : 'bg-gray-600'
      }`}
  >
    <span
      className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${enabled ? 'translate-x-6' : 'translate-x-1'
        }`}
    />
  </button>
);

// The action buttons that can appear in the player's top-right overlay. Ids
// match the gating in VideoPlayer. Undefined `player_overlay_buttons` = all on.
const OVERLAY_BUTTONS: { id: string; label: string }[] = [
  { id: 'follow', label: 'Follow / Unfollow' },
  { id: 'subscribe', label: 'Subscribe / Gift' },
  { id: 'clip', label: 'Create Clip' },
  { id: 'song', label: 'Identify Song' },
  { id: 'share', label: 'Share' },
  { id: 'clipsvods', label: 'Clips & VODs' },
  { id: 'multinook', label: 'Add to MultiNook' },
  { id: 'refresh', label: 'Refresh' },
  { id: 'close', label: 'Close Stream' },
];

const PlayerSettings = () => {
  const { settings, updateSettings } = useAppStore();

  // Fallback only used if settings.streamlink is somehow absent.
  const streamlinkDefaults = {
    stream_timeout: 60,
    retry_streams: 3,
    enhanced_codecs: true,
  };

  const streamlink = settings.streamlink || streamlinkDefaults;
  const autoSwitch = settings.auto_switch;
  const autoSwitchEnabled = autoSwitch?.enabled ?? true;
  const autoSwitchMode = autoSwitch?.mode ?? 'same_category';
  const autoSwitchNotification = autoSwitch?.show_notification ?? true;
  const autoSwitchRaid = autoSwitch?.auto_redirect_on_raid ?? true;
  const autoSwitchOfflineChat = autoSwitch?.stay_in_offline_chat ?? false;
  const videoPlayer = settings.video_player;

  // Patch a few top-level video_player fields at once.
  const setVideoPlayer = (patch: Partial<typeof videoPlayer>) => {
    updateSettings({ ...settings, video_player: { ...videoPlayer, ...patch } });
  };

  const scrollVolume = videoPlayer?.scroll_volume ?? true;
  const scrollAboutReveal = videoPlayer?.scroll_about_reveal ?? true;
  const middleClickMute = videoPlayer?.middle_click_mute ?? true;
  const resumeVodPlayback = videoPlayer?.resume_vod_playback ?? true;
  const wheelVolumeStep = videoPlayer?.wheel_volume_step ?? 0.05;
  // Both gestures want the wheel, so with both on the reveal moves to Shift.
  // The row description says so rather than leaving it to be discovered.
  const revealNeedsShift = aboutRevealNeedsShift(videoPlayer);

  // Audio boost: a compressor + makeup-gain stage on the player audio. Merge the
  // persisted values over the shared defaults so a missing/partial object still
  // renders, and write the whole object back (it persists as one nested field).
  const audioBoost = { ...DEFAULT_AUDIO_BOOST, ...(videoPlayer?.audio_boost ?? {}) };
  const setAudioBoost = (patch: Partial<typeof audioBoost>) => {
    updateSettings({
      ...settings,
      video_player: { ...videoPlayer, audio_boost: { ...audioBoost, ...patch } },
    });
  };
  // Shared fader descriptors (Boost first, then the five compressor params).
  const boostFaders = audioBoostFaderDefs(audioBoost);

  // Song identification: capture length + retry count. Merge persisted over
  // defaults and write the whole nested object back, mirroring audio boost.
  const songId = { ...DEFAULT_SONG_ID, ...(videoPlayer?.song_id ?? {}) };
  const setSongId = (patch: Partial<typeof songId>) => {
    updateSettings({
      ...settings,
      video_player: { ...videoPlayer, song_id: { ...songId, ...patch } },
    });
  };

  const setAutoSwitch = (patch: Partial<NonNullable<typeof autoSwitch>>) => {
    updateSettings({
      ...settings,
      auto_switch: {
        enabled: autoSwitchEnabled,
        mode: autoSwitchMode,
        show_notification: autoSwitchNotification,
        auto_redirect_on_raid: autoSwitchRaid,
        stay_in_offline_chat: autoSwitchOfflineChat,
        ...patch,
      },
    });
  };

  // Undefined = all buttons shown (default). Toggling one switches to an explicit
  // set; rendering order in the overlay is fixed by VideoPlayer, so set membership
  // is all that's stored.
  const isOverlayButtonOn = (id: string) =>
    !settings.player_overlay_buttons || settings.player_overlay_buttons.includes(id);
  const toggleOverlayButton = (id: string) => {
    const current = settings.player_overlay_buttons ?? OVERLAY_BUTTONS.map((b) => b.id);
    const next = current.includes(id) ? current.filter((x) => x !== id) : [...current, id];
    updateSettings({ ...settings, player_overlay_buttons: next });
  };

  return (
    <div className="space-y-8">
      <p className="text-sm text-textSecondary px-1">
        Most player controls (volume, quality, playback speed) are available directly in the video player.
        These settings control advanced streaming behavior.
      </p>

      {/* Desktop only. These toggle buttons in VideoPlayer's overlay, which the
          phone never renders: MobilePlayer has its own control set, and several
          of these (MultiNook, popout) are desktop-only features regardless. */}
      {!IS_MOBILE && (
      <SettingsSection
        label="Player Overlay Buttons"
        description="Choose which action buttons appear in the top-right of the video player. Each still only shows when it applies (Clip when clippable, MultiNook and Refresh on live streams, and so on)."
      >
        {OVERLAY_BUTTONS.map((b) => (
          <SettingsRow
            key={b.id}
            title={b.label}
            control={
              <Toggle enabled={isOverlayButtonOn(b.id)} onChange={() => toggleOverlayButton(b.id)} />
            }
          />
        ))}
      </SettingsSection>
      )}

      <SettingsSection
        id="settings-section-auto-switch"
        label="Auto-Switch"
        description="What happens when the stream you are watching ends, and where StreamNook takes you next."
      >
        <SettingsRow
          title="Move to another stream when this one ends"
          description="When the channel you are watching goes offline, StreamNook picks a new live stream and starts it for you."
          control={
            <Toggle
              enabled={autoSwitchEnabled}
              onChange={() => setAutoSwitch({ enabled: !autoSwitchEnabled })}
            />
          }
        />

        <SettingsRow
          title="Where to go next"
          description={
            autoSwitchMode === 'same_category'
              ? 'The most-watched live stream in the same game or category.'
              : 'One of your followed channels that is live right now.'
          }
          disabled={!autoSwitchEnabled}
        >
          <SegmentedSelect
            value={autoSwitchMode}
            onChange={(mode) => setAutoSwitch({ mode })}
            options={[
              { value: 'same_category', label: 'Same Category' },
              { value: 'followed_streams', label: 'Followed Streams' },
            ]}
          />
        </SettingsRow>

        <SettingsRow
          title="Tell me when it switches"
          description="A toast names the new channel each time StreamNook switches for you."
          disabled={!autoSwitchEnabled}
          control={
            <Toggle
              enabled={autoSwitchNotification}
              onChange={() => setAutoSwitch({ show_notification: !autoSwitchNotification })}
            />
          }
        />

        {/* The phone's player does not sample the picture (per-frame readback
            is the one cost a compositing-bound phone cannot afford), so the
            switch would change nothing there. */}
        {!IS_MOBILE && (
          <SettingsRow
            title="Glow with the stream"
            description="The player picks up the colour of whatever is on screen, so a neon game and a talk show don't look the same."
            control={
              <Toggle
                enabled={settings.media_glow !== false}
                onChange={() => updateSettings({ ...settings, media_glow: settings.media_glow === false })}
              />
            }
          />
        )}

        <SettingsRow
          title="Follow raids automatically"
          description="When the streamer raids another channel, StreamNook jumps there with them (you need to be signed in)."
          control={
            <Toggle
              enabled={autoSwitchRaid}
              onChange={() => setAutoSwitch({ auto_redirect_on_raid: !autoSwitchRaid })}
            />
          }
        />

        <SettingsRow
          title="Stay in chat after the stream ends"
          description="Keeps you in the channel's chat when the stream goes offline instead of switching you away."
          control={
            <Toggle
              enabled={autoSwitchOfflineChat}
              onChange={() => setAutoSwitch({ stay_in_offline_chat: !autoSwitchOfflineChat })}
            />
          }
        />
      </SettingsSection>

      <SettingsSection
        id="settings-section-streaming"
        label="Streaming"
        description="How StreamNook fetches the stream from Twitch, and how patient it is when a channel is slow to start."
      >
        <SettingsRow
          title="Allow AV1 and h265 streams"
          description="Asks Twitch for AV1 and h265 (HEVC) versions of the stream alongside h264, which some channels offer at better quality for the same bandwidth."
          help="Turn this off if you see decode errors or a black picture on older hardware. The change applies the next time a stream loads, no restart needed."
          control={
            <Toggle
              enabled={streamlink.enhanced_codecs ?? true}
              onChange={() => {
                const next = !(streamlink.enhanced_codecs ?? true);
                updateSettings({
                  ...settings,
                  streamlink: { ...streamlink, enhanced_codecs: next },
                });
                // Re-probe + report so the change takes effect on the next resolve
                // without waiting for an app restart.
                reportCodecPreference(next);
              }}
            />
          }
        />

        <SettingsRow
          title={`Keep trying for ${streamlink.stream_timeout}s`}
          description="How long StreamNook keeps trying to open a stream before giving up, which helps when a channel has only just gone live."
        >
          <input
            type="range"
            min="30"
            max="120"
            step="5"
            value={streamlink.stream_timeout}
            onChange={(e) =>
              updateSettings({
                ...settings,
                streamlink: { ...streamlink, stream_timeout: parseInt(e.target.value) },
              })
            }
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>

        <SettingsRow
          title={`Pause ${streamlink.retry_streams}s between attempts`}
          description="How long to wait between attempts while a stream is not available yet (0 means a single attempt)."
        >
          <input
            type="range"
            min="0"
            max="5"
            step="1"
            value={streamlink.retry_streams}
            onChange={(e) =>
              updateSettings({
                ...settings,
                streamlink: { ...streamlink, retry_streams: parseInt(e.target.value) },
              })
            }
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        id="settings-section-mouse-controls"
        label="Mouse Controls"
        description="Drive the player one-handed with the mouse. These apply to the main player and to each MultiNook tile."
      >
        <SettingsRow
          title="Scroll to change volume"
          description="Scroll the wheel over the player to turn the stream up and down."
          control={
            <Toggle
              enabled={scrollVolume}
              onChange={() => setVideoPlayer({ scroll_volume: !scrollVolume })}
            />
          }
        />

        <SettingsRow
          title="Scroll to open Channel About"
          description={
            revealNeedsShift
              ? "Hold Shift and scroll down over the player to slide the channel's About panel up over the stream."
              : "Scroll down over the player to slide the channel's About panel up over the stream."
          }
          help={
            revealNeedsShift
              ? "Shift is needed because the plain wheel is set to volume. Turn Scroll to change volume off and a plain scroll down opens it instead."
              : "The About pill that appears when you hover the player always opens it too."
          }
          control={
            <Toggle
              enabled={scrollAboutReveal}
              onChange={() => setVideoPlayer({ scroll_about_reveal: !scrollAboutReveal })}
            />
          }
        />

        <SettingsRow
          title="Middle-click to mute"
          description="Click the scroll wheel over the player to mute or unmute."
          control={
            <Toggle
              enabled={middleClickMute}
              onChange={() => setVideoPlayer({ middle_click_mute: !middleClickMute })}
            />
          }
        />

        <SettingsRow
          title="Resume VODs where you left off"
          description="Reopening a past broadcast picks up at your last position, and Home keeps a Continue Watching row."
          help="Off starts every VOD from the beginning and hides the Continue Watching row; positions are still remembered for the video cards. Only VODs you open yourself are remembered, never a live stream you were watching."
          control={
            <Toggle
              enabled={resumeVodPlayback}
              onChange={() => {
                setVideoPlayer({ resume_vod_playback: !resumeVodPlayback });
                // The row is gated on this setting, and Home is mounted behind
                // this dialog, so ask Rust to rebuild it rather than waiting
                // for a remount.
                void invoke('refresh_home_section', { section: 'continue_watching' }).catch(() => {});
              }}
            />
          }
        />

        <SettingsRow
          title="Volume Step"
          description="How far one wheel notch moves the volume."
          disabled={!scrollVolume}
        >
          <div className="flex items-center gap-3">
            <input
              type="range"
              min="1"
              max="25"
              step="1"
              value={Math.round(wheelVolumeStep * 100)}
              onChange={(e) => setVideoPlayer({ wheel_volume_step: parseInt(e.target.value) / 100 })}
              className="w-full accent-accent cursor-pointer"
            />
            <span className="text-[12px] font-medium text-textPrimary tabular-nums flex-shrink-0">
              {Math.round(wheelVolumeStep * 100)}%
            </span>
          </div>
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        id="settings-section-video-player"
        label="Video Player"
        description="How streams start and how the player buffers; you can still change most of this from the player itself."
      >
        <SettingsRow
          title="Play as soon as a stream opens"
          description="The stream starts playing the moment it loads, with no need to press play."
          control={
            <Toggle
              enabled={videoPlayer?.autoplay ?? true}
              onChange={() =>
                updateSettings({
                  ...settings,
                  video_player: { ...videoPlayer, autoplay: !(videoPlayer?.autoplay ?? true) },
                })
              }
            />
          }
        />

        {/* Phone-only: a desktop has nothing to leave the app INTO, and no
            picture-in-picture window to choose against. */}
        {IS_MOBILE && (
          <SettingsRow
            title="Keep playing in the background"
            description={
              (videoPlayer?.background_mode ?? 'pip') === 'audio'
                ? 'Leaving the app keeps the audio going behind a media notification. Tap it to come back.'
                : 'Leaving the app floats the video in a picture-in-picture window.'
            }
            control={
              <Toggle
                // On = audio only. Framed around the audio because that is what
                // the viewer is choosing; losing the floating window is the
                // consequence, not the point.
                enabled={(videoPlayer?.background_mode ?? 'pip') === 'audio'}
                onChange={() =>
                  updateSettings({
                    ...settings,
                    video_player: {
                      ...videoPlayer,
                      background_mode:
                        (videoPlayer?.background_mode ?? 'pip') === 'audio' ? 'pip' : 'audio',
                    },
                  })
                }
              />
            }
          />
        )}

        <SettingsRow
          title="How close to live to stay"
          description="How far behind the live edge the player rides; lower is closer to live. Auto picks a gap that suits each channel (reopen the stream to apply)."
          help={`Auto rides ${AUTO_GAP.ll.toFixed(1)}s behind on channels the low-latency engine serves, ${AUTO_GAP.promotion.toFixed(1)}s on other low-latency broadcasts and ${AUTO_GAP.plain.toFixed(1)}s on normal-latency ones. Set your own number to override all three; the lowest gaps need a solid connection to stay smooth.`}
        >
          <div className="flex items-center gap-3">
            <input
              type="range"
              min="2"
              max="10"
              step="0.1"
              value={videoPlayer?.ll_target_latency ?? AUTO_GAP.ll}
              onChange={(e) =>
                updateSettings({
                  ...settings,
                  video_player: { ...videoPlayer, ll_target_latency: parseFloat(e.target.value) },
                })
              }
              className="w-full accent-accent cursor-pointer"
            />
            <span className="text-[12px] font-medium text-textPrimary tabular-nums flex-shrink-0">
              {videoPlayer?.ll_target_latency == null ? 'Auto' : `${videoPlayer.ll_target_latency.toFixed(1)}s`}
            </span>
            {videoPlayer?.ll_target_latency != null && (
              <button
                type="button"
                onClick={() =>
                  updateSettings({
                    ...settings,
                    video_player: { ...videoPlayer, ll_target_latency: null },
                  })
                }
                className="glass-button flex-shrink-0 rounded px-2 py-0.5 text-[11px] font-medium text-textSecondary hover:text-textPrimary"
              >
                Auto
              </button>
            )}
          </div>
        </SettingsRow>

        <SettingsRow
          title="Low Latency"
          description="Rides as close to live as the Twitch site on channels that support it. On by default."
          help="Off falls back to the wider whole-segment path. If a stream stutters or refuses to play, turn this off first."
          control={
            <Toggle
              enabled={videoPlayer?.experimental_low_latency ?? true}
              onChange={() => {
                const next = !(videoPlayer?.experimental_low_latency ?? true);
                updateSettings({
                  ...settings,
                  video_player: { ...videoPlayer, experimental_low_latency: next },
                });
                invoke('set_experimental_low_latency', { enabled: next }).catch(() => {});
              }}
            />
          }
        />

        <SettingsRow
          title={`Buffer up to ${videoPlayer?.max_buffer_length ?? 120}s ahead`}
          description="How much video the player keeps loaded ahead of playback; more is steadier on a shaky connection but adds delay."
        >
          <input
            type="range"
            min="3"
            max="300"
            step="1"
            value={videoPlayer?.max_buffer_length ?? 120}
            onChange={(e) =>
              updateSettings({
                ...settings,
                video_player: {
                  ...videoPlayer,
                  max_buffer_length: parseInt(e.target.value),
                },
              })
            }
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>

        <SettingsRow
          title="Quality to start streams at"
          description="Every stream opens at this quality, and you can change it anytime from the player controls."
        >
          <Dropdown
            value={settings.quality}
            onChange={(v) => updateSettings({ ...settings, quality: v })}
            className="w-full"
            ariaLabel="Default stream quality"
            options={[
              { value: 'best', label: 'Auto (Source)' },
              { value: '1440p60', label: '1440p60' },
              { value: '1080p60', label: '1080p60' },
              { value: '720p60', label: '720p60' },
              { value: '480p30', label: '480p30' },
              { value: '360p30', label: '360p30' },
              { value: '160p30', label: '160p30' },
              { value: 'audio_only', label: 'Audio Only' },
            ]}
          />
        </SettingsRow>

        {/* Desktop only: it constrains WINDOW RESIZE, and there is no resizable
            window here. The phone's player band is a fixed 16:9 already. */}
        {!IS_MOBILE && (
          <SettingsRow
            title="Keep the window at 16:9"
            description="Resizing the window snaps to the video's shape, so you never see black bars around the picture."
            control={
              <Toggle
                enabled={videoPlayer?.lock_aspect_ratio ?? true}
                onChange={() =>
                  updateSettings({
                    ...settings,
                    video_player: { ...videoPlayer, lock_aspect_ratio: !(videoPlayer?.lock_aspect_ratio ?? true) },
                  })
                }
              />
            }
          />
        )}

        {/* Desktop only: `cinema_mode` is read solely by VideoPlayer.tsx, which
            the phone shell never renders, so the toggle did nothing here. */}
        {!IS_MOBILE && (
          <SettingsRow
            title="Cinema Mode"
            description="Use solid black letterbox bars. When off, the bars match your theme color so the video appears to float."
            control={
              <Toggle
                enabled={videoPlayer?.cinema_mode ?? false}
                onChange={() =>
                  updateSettings({
                    ...settings,
                    video_player: { ...videoPlayer, cinema_mode: !(videoPlayer?.cinema_mode ?? false) },
                  })
                }
              />
            }
          />
        )}

        <SettingsRow
          title="Start streams muted"
          description="Every stream opens silent until you unmute it."
          control={
            <Toggle
              enabled={videoPlayer?.muted ?? false}
              onChange={() =>
                updateSettings({
                  ...settings,
                  video_player: { ...videoPlayer, muted: !(videoPlayer?.muted ?? false) },
                })
              }
            />
          }
        />

        <SettingsRow
          title={`Starting volume: ${Math.round((videoPlayer?.volume ?? 1.0) * 100)}%`}
          description="The volume every stream opens at before you adjust it."
        >
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={videoPlayer?.volume ?? 1.0}
            onChange={(e) =>
              updateSettings({
                ...settings,
                video_player: { ...videoPlayer, volume: parseFloat(e.target.value) },
              })
            }
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>
      </SettingsSection>

      {/* Desktop only, for now. `applyAudioBoost` is called from exactly one
          place - VideoPlayer.tsx - and the phone renders MobilePlayer, so every
          control in here was inert on Android. Worth WIRING rather than
          dropping at some point: phone speakers are quiet and this is the
          setting that would help most. It is a small change (call
          applyAudioBoost on the mobile <video> the same way), just not one to
          make blind inside a settings audit. */}
      {!IS_MOBILE && (
      <SettingsSection
        id="settings-section-audio-boost"
        label="Audio Boost"
        description={`Even out loud and quiet moments and push the stream a little louder than the source, without the harsh clipping you get from raising volume past 100%.${
          AUDIO_GRAPH_SUPPORTED ? '' : ` ${AUDIO_GRAPH_REFUSAL}`
        }`}
      >
        {/* Dimmed rather than hidden on the shells that cannot run the audio
            graph, for the same reason the MultiNook overlay button is: a
            feature that silently vanishes reads as a bug, while a dimmed
            control with the reason above it reads as a limitation. The wrapper
            is a plain box so the rows keep the card's own padding maths. */}
        <div className={AUDIO_GRAPH_SUPPORTED ? '' : 'opacity-50 pointer-events-none'}>
        <SettingsRow
          title="Turn on Audio Boost"
          description="Evens out the stream's loudness and lifts it, on top of the normal volume slider."
          help="Under the hood the audio runs through a compressor and then a makeup-gain stage. The defaults give a clean, balanced lift; fine-tune below to taste."
          control={
            <Toggle
              enabled={audioBoost.enabled}
              onChange={() => setAudioBoost({ enabled: !audioBoost.enabled })}
            />
          }
        />

        <SettingsRow
          title="Boost"
          description="How much louder to make the stream after compression. 100% is no extra boost; higher is louder."
          disabled={!audioBoost.enabled}
        >
          <div className="flex justify-center pt-1">
            <Fader
              label={boostFaders[0].label}
              display={boostFaders[0].display}
              value={boostFaders[0].value}
              min={boostFaders[0].min}
              max={boostFaders[0].max}
              step={boostFaders[0].step}
              onChange={(v) => setAudioBoost(boostFaders[0].apply(v))}
            />
          </div>
        </SettingsRow>

        <SettingsRow
          title="Advanced Compressor Controls"
          description="Shape exactly how the compressor responds. Hover a label for what it does; the defaults are a gentle, natural starting point."
          disabled={!audioBoost.enabled}
        >
          <details className="group">
            <summary className="cursor-pointer text-sm font-medium text-textSecondary hover:text-textPrimary transition-colors flex items-center gap-2">
              <span className="transform transition-transform group-open:rotate-90">▶</span>
              Show advanced controls
            </summary>
            <div className="mt-5 flex flex-wrap items-end justify-center gap-x-6 gap-y-6">
              {boostFaders.slice(1).map((d) => (
                <Fader
                  key={d.key}
                  label={d.label}
                  display={d.display}
                  value={d.value}
                  min={d.min}
                  max={d.max}
                  step={d.step}
                  hint={d.hint}
                  onChange={(v) => setAudioBoost(d.apply(v))}
                />
              ))}
            </div>

            <div className="mt-5 flex justify-center">
              <button
                onClick={() => setAudioBoost(audioBoostResetPatch())}
                style={{ borderRadius: 8 }}
                className="glass-button text-textSecondary hover:text-textPrimary text-sm px-3 py-2"
              >
                Reset to defaults
              </button>
            </div>
          </details>
        </SettingsRow>
        </div>
      </SettingsSection>
      )}

      <SettingsSection
        id="settings-section-song-id"
        label="Song Identification"
        description={`The /song command and the player's music button listen to a few seconds of the stream and name the track.${
          AUDIO_GRAPH_SUPPORTED ? '' : ` ${AUDIO_GRAPH_REFUSAL}`
        }`}
      >
        {/* Recognition listens through the same tap Audio Boost uses, so it is
            closed on exactly the same shells. */}
        <div className={AUDIO_GRAPH_SUPPORTED ? '' : 'opacity-50 pointer-events-none'}>
        <SettingsRow
          title={`Listen for ${songId.capture_seconds}s`}
          description="How many seconds of audio to fingerprint; longer matches more reliably over talking or noise, but the result takes a little longer to appear."
        >
          <input
            type="range"
            min="3"
            max="20"
            step="1"
            value={songId.capture_seconds}
            onChange={(e) => setSongId({ capture_seconds: parseInt(e.target.value, 10) })}
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>

        <SettingsRow
          title={`Retries when nothing matches: ${songId.retries}`}
          description="If the first listen finds nothing, StreamNook listens again this many times."
          help="Each retry listens to a fresh window, so an ad break or quiet moment gets another shot."
        >
          <input
            type="range"
            min="0"
            max="3"
            step="1"
            value={songId.retries}
            onChange={(e) => setSongId({ retries: parseInt(e.target.value, 10) })}
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>
        </div>
      </SettingsSection>
    </div>
  );
};

export default PlayerSettings;
