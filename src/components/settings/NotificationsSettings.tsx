import { useState } from 'react';
import { Dropdown } from '../ui/Dropdown';
import { Tooltip } from '../ui/Tooltip';
import { Toggle } from '../ui/Toggle';
import { useAppStore } from '../../stores/AppStore';
import {
  ToastPosition,
  DEFAULT_TOAST_POSITION,
  DEFAULT_TOAST_EDGE_OFFSET,
} from '../../types';
import { invoke } from '@tauri-apps/api/core';
import { Award, Bell, Gift } from 'lucide-react';
import { SettingsSection, SettingsRow } from './_primitives';
import { SoundVolume } from './SoundControls';
import { useSettingReset } from './settingReset';
import { InlineSlider, SubControl, SubControls } from '../plugins/settingsPageKit';
import { useSoundOptions } from '../../hooks/useSoundOptions';
import { grantAccolade } from '../../services/supabaseService';
import {
  RESTLESS_ACCOLADE_ID,
  NOTIF_CLICK_THRESHOLD,
  bumpTestNotificationClicks,
} from '../../utils/notifAchievement';

import { Logger } from '../../utils/logger';

// The six supported toast anchors, placed on a 3x3 grid that stands in for the
// screen. The middle row's sides are intentionally inert (vertically-centered
// toasts would overlap the video), leaving only the screen-center marker there.
const TOAST_ANCHORS: { value: ToastPosition; col: number; row: number; label: string }[] = [
  { value: 'top-left', col: 1, row: 1, label: 'Top left' },
  { value: 'top-center', col: 2, row: 1, label: 'Top center' },
  { value: 'top-right', col: 3, row: 1, label: 'Top right' },
  { value: 'bottom-left', col: 1, row: 3, label: 'Bottom left' },
  { value: 'bottom-center', col: 2, row: 3, label: 'Bottom center' },
  { value: 'bottom-right', col: 3, row: 3, label: 'Bottom right' },
];

// Edge-spacing slider bounds, shared with the visualization math below.
const EDGE_OFFSET_MIN = 8;
const EDGE_OFFSET_MAX = 200;
// How far (px) the selected slot lifts toward center across the full spacing
// range, so dragging the spacing slider visibly moves the toast off its edge.
const PICKER_MAX_TRAVEL = 22;

const ToastPositionPicker = ({
  value,
  offset,
  onChange,
}: {
  value: ToastPosition;
  offset: number;
  onChange: (v: ToastPosition) => void;
}) => {
  const currentLabel = TOAST_ANCHORS.find((a) => a.value === value)?.label ?? '';
  const travelFraction = Math.max(
    0,
    Math.min(1, (offset - EDGE_OFFSET_MIN) / (EDGE_OFFSET_MAX - EDGE_OFFSET_MIN)),
  );
  return (
    <div className="flex items-center gap-4">
      <div className="glass-input rounded-lg p-2.5" style={{ width: 200 }}>
        <div className="relative grid grid-cols-3 grid-rows-3 gap-1" style={{ aspectRatio: '16 / 10' }}>
          {/* faint screen-center marker so the box reads as a screen */}
          <div className="col-start-2 row-start-2 flex items-center justify-center">
            <span className="block w-1 h-1 rounded-full bg-textMuted/30" />
          </div>
          {TOAST_ANCHORS.map((anchor) => {
            const active = value === anchor.value;
            // Top anchors lift downward (away from the top edge), bottom anchors
            // lift upward; only the selected slot moves, mirroring edge spacing.
            const isTop = anchor.row === 1;
            const lift = travelFraction * PICKER_MAX_TRAVEL * (isTop ? 1 : -1);
            return (
              <Tooltip key={anchor.value} content={anchor.label} side="top">
                <button
                  type="button"
                  onClick={() => onChange(anchor.value)}
                  aria-label={anchor.label}
                  aria-pressed={active}
                  style={{ gridColumn: anchor.col, gridRow: anchor.row }}
                  className={`group flex items-center justify-center rounded-md cursor-pointer transition-colors ${
                    active ? 'bg-accent/10' : 'hover:bg-glass'
                  }`}
                >
                  {/* Every spot shows a toast-shaped slot so the available choices
                      are obvious at rest; the selected one fills with accent and
                      lifts off its edge to preview the current edge spacing. */}
                  <span
                    style={active ? { transform: `translateY(${lift}px)` } : undefined}
                    className={`block rounded-[3px] border transition-all ${
                      active
                        ? 'w-6 h-2.5 bg-accent border-accent'
                        : 'w-5 h-2 bg-textMuted/15 border-textMuted/50 group-hover:w-6 group-hover:h-2.5 group-hover:bg-accent/25 group-hover:border-accent/70'
                    }`}
                  />
                </button>
              </Tooltip>
            );
          })}
        </div>
      </div>
      <div className="text-[12px] text-textSecondary">
        <span className="font-medium text-textPrimary">{currentLabel}</span>
        <span className="block text-[11px] text-textMuted mt-0.5">Click a spot to move toasts there</span>
      </div>
    </div>
  );
};

const NotificationsSettings = () => {
  const { settings, updateSettings, currentUser, addToast } = useAppStore();
  const resetFor = useSettingReset();
  // Brief green flash on press, used as the Test button's feedback instead of a
  // text/width swap. See handleTestNotification + the button's className.
  const [testFlash, setTestFlash] = useState(false);

  const liveNotifications = settings.live_notifications || {
    enabled: true,
    play_sound: true,
    show_live_notifications: true,
    show_favorite_live_notifications: true,
    show_whisper_notifications: true,
    show_update_notifications: true,
    show_drops_notifications: true,
    show_favorite_drops_notifications: true,
    show_channel_points_notifications: true,
    show_badge_notifications: true,
    show_gift_sub_notifications: true,
    show_twitch_reward_notifications: true,
    use_dynamic_island: true,
    use_toast: true,
    toast_position: DEFAULT_TOAST_POSITION,
    toast_edge_offset: DEFAULT_TOAST_EDGE_OFFSET,
  };

  const soundOptions = useSoundOptions({ defaultId: 'boop' });

  const updateLiveNotifications = (updates: Partial<typeof liveNotifications>) => {
    updateSettings({
      ...settings,
      live_notifications: {
        ...liveNotifications,
        ...updates,
      },
    });
  };

  // Dev builds only. Gift subs arrive days apart, so there is otherwise no
  // way to look at that row on demand. Prefers a real one off the feed and
  // falls back to a stand-in, so it also proves the query works end to end.
  const handleGiftSubPreview = async () => {
    try {
      await invoke('send_test_notification', { kind: 'gift_sub' });
    } catch (error) {
      Logger.error('Failed to preview gift sub notification:', error);
      addToast('Gift sub preview failed, see the log', 'error');
    }
  };

  // Dev builds only: the newest real reward row off your Twitch feed.
  const handleRewardPreview = async () => {
    try {
      await invoke('send_test_notification', { kind: 'twitch_reward' });
    } catch (error) {
      Logger.error('Failed to preview reward notification:', error);
      addToast(`Reward preview failed: ${error}`, 'error');
    }
  };

  const handleTestNotification = async () => {
    // Snap to green, then let it fade back to the glass surface (the fade-back
    // lives on the glass-button class transition). Brief hold so it reads as a flash.
    setTestFlash(true);
    setTimeout(() => setTestFlash(false), 150);
    try {
      await invoke('send_test_notification');
      // "Insomniac" easter egg: reward the stubborn Test-clicker. The running
      // count is local; on the 50th click persist the accolade (which also
      // unlocks the Midnight Atmosphere for any member, no sub required) and
      // celebrate exactly once.
      const clicks = bumpTestNotificationClicks();
      if (clicks === NOTIF_CLICK_THRESHOLD && currentUser?.user_id) {
        void grantAccolade(currentUser.user_id, RESTLESS_ACCOLADE_ID);
        addToast('Achievement unlocked: Restless. The Midnight Atmosphere is yours.', 'success', undefined, { alwaysShow: true });
      }
    } catch (error) {
      Logger.error('Failed to send test notification:', error);
    }
  };

  return (
    <div className="space-y-8">
      <SettingsSection
        label="Notifications"
        description="Turn everything on or off here, then choose where alerts show and which ones you want below."
      >
        <SettingsRow
          title="All notifications"
          onReset={resetFor(['live_notifications.enabled', true])}
          description="Turn this off to silence every notification at once; your choices below stay saved for when you turn it back on."
          control={
            <Toggle
              enabled={liveNotifications.enabled}
              onChange={() => updateLiveNotifications({ enabled: !liveNotifications.enabled })}
            />
          }
        />
      </SettingsSection>

      {liveNotifications.enabled && (
        <>
          <SettingsSection
            label="Display"
            description="Where notifications show up: in the Dynamic Island at the top of the window, as toast popups, or both."
          >
            <SettingsRow
              title="Dynamic Island"
              onReset={resetFor(['live_notifications.use_dynamic_island', true])}
              description="Notifications appear in the notification center at the top of the window."
              control={
                <Toggle
                  enabled={liveNotifications.use_dynamic_island ?? true}
                  onChange={() => updateLiveNotifications({
                    use_dynamic_island: !(liveNotifications.use_dynamic_island ?? true)
                  })}
                />
              }
            />

            <SettingsRow
              title="Toasts"
              onReset={resetFor(['live_notifications.use_toast', true])}
              description="Each notification also pops up as a small card at the edge of the window."
              help="Position: click a spot on the mini screen to move toasts to that corner or edge. Edge distance is how far they sit from the top or bottom edge of the window; raise it to push them further in."
              control={
                <Toggle
                  enabled={liveNotifications.use_toast ?? true}
                  onChange={() => updateLiveNotifications({
                    use_toast: !(liveNotifications.use_toast ?? true)
                  })}
                />
              }
            >
              {(liveNotifications.use_toast ?? true) && (
                <SubControls>
                  <SubControl
                    title="Position"
                    onReset={resetFor(['live_notifications.toast_position', DEFAULT_TOAST_POSITION])}
                    stacked
                    control={
                      <ToastPositionPicker
                        value={liveNotifications.toast_position ?? DEFAULT_TOAST_POSITION}
                        offset={liveNotifications.toast_edge_offset ?? DEFAULT_TOAST_EDGE_OFFSET}
                        onChange={(toast_position) => updateLiveNotifications({ toast_position })}
                      />
                    }
                  />
                  <SubControl
                    title="Edge distance"
                    onReset={resetFor(['live_notifications.toast_edge_offset', DEFAULT_TOAST_EDGE_OFFSET])}
                    control={
                      <InlineSlider
                        value={liveNotifications.toast_edge_offset ?? DEFAULT_TOAST_EDGE_OFFSET}
                        min={EDGE_OFFSET_MIN}
                        max={EDGE_OFFSET_MAX}
                        step={4}
                        label="Toast distance from the edge"
                        format={(v) => `${v}px`}
                        onChange={(toast_edge_offset) => updateLiveNotifications({ toast_edge_offset })}
                      />
                    }
                  />
                </SubControls>
              )}
            </SettingsRow>
          </SettingsSection>

          <SettingsSection
            label="Notification Types"
            description="Pick which events are worth a notification."
          >
            <SettingsRow
              title="Followed channels"
              onReset={resetFor(['live_notifications.show_live_notifications', true])}
              description="When a channel you follow goes live, the moment it starts streaming. Clicking it opens the stream."
              control={
                <Toggle
                  enabled={liveNotifications.show_live_notifications ?? true}
                  onChange={() => updateLiveNotifications({
                    show_live_notifications: !(liveNotifications.show_live_notifications ?? true)
                  })}
                />
              }
            />

            <SettingsRow
              title="Favorite channels"
              onReset={resetFor(['live_notifications.show_favorite_live_notifications', true])}
              description="When a favorite channel goes live, even one you do not follow on Twitch."
              control={
                <Toggle
                  enabled={liveNotifications.show_favorite_live_notifications ?? true}
                  onChange={() => updateLiveNotifications({
                    show_favorite_live_notifications: !(liveNotifications.show_favorite_live_notifications ?? true)
                  })}
                />
              }
            />

            <SettingsRow
              title="Whispers"
              onReset={resetFor(['live_notifications.show_whisper_notifications', true])}
              description="A notification shows each new whisper, and clicking it opens the conversation."
              control={
                <Toggle
                  enabled={liveNotifications.show_whisper_notifications ?? true}
                  onChange={() => updateLiveNotifications({
                    show_whisper_notifications: !(liveNotifications.show_whisper_notifications ?? true)
                  })}
                />
              }
            />

            {/* The notification opens the changelog popup on the new
                version, with Install beside the notes. Turning this row off
                still leaves the title-bar button and Check for updates in the
                changelog, which is what the description says. */}
            <SettingsRow
              title="App updates"
              onReset={resetFor(['live_notifications.show_update_notifications', true])}
              description="A notification tells you when a new StreamNook version is out, and opens the changelog to what is in it. Turning this off leaves the update button in the title bar and Check for updates in the changelog."
              control={
                <Toggle
                  enabled={liveNotifications.show_update_notifications ?? true}
                  onChange={() => updateLiveNotifications({
                    show_update_notifications: !(liveNotifications.show_update_notifications ?? true)
                  })}
                />
              }
            />

            <SettingsRow
              title="Claimed drops"
              onReset={resetFor(['live_notifications.show_drops_notifications', true])}
              description="A notification confirms each drop StreamNook claims for you."
              help="New drops: at startup, StreamNook checks your favorite categories and tells you when they have new drops to earn."
              control={
                <Toggle
                  enabled={liveNotifications.show_drops_notifications ?? true}
                  onChange={() => updateLiveNotifications({
                    show_drops_notifications: !(liveNotifications.show_drops_notifications ?? true)
                  })}
                />
              }
            >
              {(liveNotifications.show_drops_notifications ?? true) && (
                <SubControls>
                  <SubControl
                    title="New drops"
                    onReset={resetFor(['live_notifications.show_favorite_drops_notifications', true])}
                    control={
                      <Toggle
                        enabled={liveNotifications.show_favorite_drops_notifications ?? true}
                        onChange={() => updateLiveNotifications({
                          show_favorite_drops_notifications: !(liveNotifications.show_favorite_drops_notifications ?? true)
                        })}
                      />
                    }
                  />
                </SubControls>
              )}
            </SettingsRow>

            <SettingsRow
              title="Channel points"
              onReset={resetFor(['live_notifications.show_channel_points_notifications', true])}
              description="A notification confirms each channel points bonus claimed for you."
              control={
                <Toggle
                  enabled={liveNotifications.show_channel_points_notifications ?? true}
                  onChange={() => updateLiveNotifications({
                    show_channel_points_notifications: !(liveNotifications.show_channel_points_notifications ?? true)
                  })}
                />
              }
            />

            <SettingsRow
              title="New badges"
              onReset={resetFor(['live_notifications.show_badge_notifications', true])}
              description="You hear about new badges as soon as they become available to earn."
              control={
                <Toggle
                  enabled={liveNotifications.show_badge_notifications ?? true}
                  onChange={() => updateLiveNotifications({
                    show_badge_notifications: !(liveNotifications.show_badge_notifications ?? true)
                  })}
                />
              }
            />

            <SettingsRow
              title="Gift subs"
              onReset={resetFor(['live_notifications.show_gift_sub_notifications', true])}
              description="You hear about gift subs you receive, even for a channel you were not watching at the time."
              control={
                <Toggle
                  enabled={liveNotifications.show_gift_sub_notifications ?? true}
                  onChange={() => updateLiveNotifications({
                    show_gift_sub_notifications: !(liveNotifications.show_gift_sub_notifications ?? true)
                  })}
                />
              }
            />

            <SettingsRow
              title="Earned rewards"
              onReset={resetFor(['live_notifications.show_twitch_reward_notifications', true])}
              description="When Twitch names a reward for you: which badge you earned or which drop reward is waiting, by name, however it was earned."
              control={
                <Toggle
                  enabled={liveNotifications.show_twitch_reward_notifications ?? true}
                  onChange={() => updateLiveNotifications({
                    show_twitch_reward_notifications: !(liveNotifications.show_twitch_reward_notifications ?? true)
                  })}
                />
              }
            />
          </SettingsSection>

          <SettingsSection
            label="Sound"
            description="A quiet sound with each notification, if you want one."
          >
            <SettingsRow
              title="Sound"
              onReset={resetFor(['live_notifications.play_sound', true])}
              description="Plays a soft sound with each notification."
              help="Every tone is soft and short, so none of them will startle you. Volume: let go of the slider to hear it."
              control={
                <Toggle
                  enabled={liveNotifications.play_sound}
                  onChange={() => updateLiveNotifications({ play_sound: !liveNotifications.play_sound })}
                />
              }
            >
              {liveNotifications.play_sound && (
                <SubControls>
                  <SubControl
                    title="Tone"
                    onReset={resetFor(['live_notifications.sound_type', 'boop'])}
                    control={
                      <Dropdown
                        value={liveNotifications.sound_type || 'boop'}
                        onChange={(v) => updateLiveNotifications({ sound_type: v })}
                        className="w-44"
                        ariaLabel="Notification sound"
                        options={soundOptions}
                      />
                    }
                  />
                  <SubControl
                    title="Volume"
                    onReset={resetFor(['live_notifications.sound_volume', 100])}
                    control={
                      <SoundVolume
                        value={liveNotifications.sound_volume ?? 100}
                        onChange={(sound_volume) => updateLiveNotifications({ sound_volume })}
                        sound={liveNotifications.sound_type || 'boop'}
                        label="Notification sound volume"
                      />
                    }
                  />
                </SubControls>
              )}
            </SettingsRow>

            <SettingsRow
              title="Test notification"
              description="Fires a sample notification so you can check the position, sound, and style you picked."
              control={
                <button
                  onClick={handleTestNotification}
                  // Frosted glass-button (bevel + accent-tinted surface) to match
                  // the rest of the app, not the old flat accent fill. Press
                  // feedback is a subtle green flash: an inline green snaps on
                  // (transition: none), then the glass-button's own transition
                  // fades it back to the surface color.
                  className="glass-button flex items-center gap-2 px-4 py-2 rounded-lg text-textPrimary text-sm font-medium"
                  style={testFlash ? { backgroundColor: 'color-mix(in srgb, var(--color-success) 50%, var(--glass-under, transparent))', transition: 'none' } : undefined}
                >
                  <Bell size={16} />
                  Test
                </button>
              }
            />

            {import.meta.env.DEV && (
              <SettingsRow
                title="Gift sub preview"
                description="Development builds only. Shows the gift sub row using a real one from your Twitch feed when there is one."
                control={
                  <button
                    onClick={handleGiftSubPreview}
                    className="glass-button flex items-center gap-2 px-4 py-2 rounded-lg text-textPrimary text-sm font-medium"
                  >
                    <Gift size={16} />
                    Preview
                  </button>
                }
              />
            )}

            {import.meta.env.DEV && (
              <SettingsRow
                title="Reward preview"
                description="Development builds only. Shows the newest real badge or drop reward from your Twitch feed."
                control={
                  <button
                    onClick={handleRewardPreview}
                    className="glass-button flex items-center gap-2 px-4 py-2 rounded-lg text-textPrimary text-sm font-medium"
                  >
                    <Award size={16} />
                    Preview
                  </button>
                }
              />
            )}
          </SettingsSection>
        </>
      )}

      <SettingsSection label="About" bare>
        <div className="glass-panel p-3 rounded-lg border border-accent/20">
          <div className="flex items-start gap-3">
            <div className="text-accent mt-0.5">
              <svg
                className="w-5 h-5"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                />
              </svg>
            </div>
            <div className="flex-1">
              <p className="text-xs text-textPrimary font-medium mb-1">
                About Notifications
              </p>
              <p className="text-xs text-textSecondary">
                Notifications can show in the Dynamic Island (the notification center at the top), as toast popups at any corner or edge you like, or both. Clicking one takes action: a live notification starts the stream, a whisper opens the conversation, and an update takes you to the Updates page.
              </p>
            </div>
          </div>
        </div>
      </SettingsSection>
    </div>
  );
};

export default NotificationsSettings;
