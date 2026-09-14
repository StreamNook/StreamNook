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
import { Bell } from 'lucide-react';
import { SettingsSection, SettingsRow } from './_primitives';
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
    use_dynamic_island: true,
    use_toast: true,
    toast_position: DEFAULT_TOAST_POSITION,
    toast_edge_offset: DEFAULT_TOAST_EDGE_OFFSET,
  };

  const updateLiveNotifications = (updates: Partial<typeof liveNotifications>) => {
    updateSettings({
      ...settings,
      live_notifications: {
        ...liveNotifications,
        ...updates,
      },
    });
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
          title="Show notifications"
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
            label="Notification Methods"
            description="Where notifications show up: in the Dynamic Island at the top of the window, as toast popups, or both."
          >
            <SettingsRow
              title="Show in the Dynamic Island"
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
              title="Show toast popups"
              description="Each notification also pops up as a small card at the edge of the window you choose below."
              control={
                <Toggle
                  enabled={liveNotifications.use_toast ?? true}
                  onChange={() => updateLiveNotifications({
                    use_toast: !(liveNotifications.use_toast ?? true)
                  })}
                />
              }
            />

            {(liveNotifications.use_toast ?? true) && (
              <>
                <SettingsRow
                  title="Where toasts appear"
                  description="Click a spot on the mini screen to move toasts to that corner or edge."
                >
                  <ToastPositionPicker
                    value={liveNotifications.toast_position ?? DEFAULT_TOAST_POSITION}
                    offset={liveNotifications.toast_edge_offset ?? DEFAULT_TOAST_EDGE_OFFSET}
                    onChange={(toast_position) => updateLiveNotifications({ toast_position })}
                  />
                </SettingsRow>

                <SettingsRow
                  title={`Distance from the edge: ${liveNotifications.toast_edge_offset ?? DEFAULT_TOAST_EDGE_OFFSET}px`}
                  description="How far toasts sit from the top or bottom edge of the window; raise it to push them further in."
                >
                  <input
                    type="range"
                    min={EDGE_OFFSET_MIN}
                    max={EDGE_OFFSET_MAX}
                    step="4"
                    value={liveNotifications.toast_edge_offset ?? DEFAULT_TOAST_EDGE_OFFSET}
                    onChange={(e) =>
                      updateLiveNotifications({ toast_edge_offset: parseInt(e.target.value) })
                    }
                    className="w-full accent-accent cursor-pointer"
                  />
                </SettingsRow>
              </>
            )}
          </SettingsSection>

          <SettingsSection
            label="Notification Types"
            description="Pick which events are worth a notification."
          >
            <SettingsRow
              title="When a followed channel goes live"
              description="You get a notification the moment someone you follow starts streaming, and clicking it opens the stream."
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
              title="When a favorite channel goes live"
              description="Channels you have favorited notify you even if you do not follow them on Twitch."
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
              title="When you get a whisper"
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

            <SettingsRow
              title="When an app update is ready"
              description="You hear about new StreamNook versions as soon as they are available, and clicking takes you to the Updates page."
              control={
                <Toggle
                  enabled={liveNotifications.show_update_notifications ?? true}
                  onChange={() => updateLiveNotifications({
                    show_update_notifications: !(liveNotifications.show_update_notifications ?? true)
                  })}
                />
              }
            />

            {(liveNotifications.show_update_notifications ?? true) && (liveNotifications.use_toast ?? true) && (
              <SettingsRow
                title="Update straight from the toast"
                description="Clicking the update toast starts installing right away instead of opening the Updates page first."
                control={
                  <Toggle
                    enabled={liveNotifications.quick_update_on_toast ?? false}
                    onChange={() => updateLiveNotifications({
                      quick_update_on_toast: !(liveNotifications.quick_update_on_toast ?? false)
                    })}
                  />
                }
              />
            )}

            <SettingsRow
              title="When a drop is claimed"
              description="A notification confirms each drop StreamNook claims for you."
              control={
                <Toggle
                  enabled={liveNotifications.show_drops_notifications ?? true}
                  onChange={() => updateLiveNotifications({
                    show_drops_notifications: !(liveNotifications.show_drops_notifications ?? true)
                  })}
                />
              }
            />

            {(liveNotifications.show_drops_notifications ?? true) && (
              <SettingsRow
                title="New drops in favorite categories"
                description="At startup, StreamNook checks your favorite categories and tells you when they have new drops to earn."
                control={
                  <Toggle
                    enabled={liveNotifications.show_favorite_drops_notifications ?? true}
                    onChange={() => updateLiveNotifications({
                      show_favorite_drops_notifications: !(liveNotifications.show_favorite_drops_notifications ?? true)
                    })}
                  />
                }
              />
            )}

            <SettingsRow
              title="When channel points are claimed"
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
              title="When new badges appear"
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
          </SettingsSection>

          <SettingsSection
            label="Sound"
            description="A quiet sound with each notification, if you want one."
          >
            <SettingsRow
              title="Play a sound"
              description="A soft sound plays with each notification, in the style you pick below."
              control={
                <Toggle
                  enabled={liveNotifications.play_sound}
                  onChange={() => updateLiveNotifications({ play_sound: !liveNotifications.play_sound })}
                />
              }
            />

            {liveNotifications.play_sound && (
              <SettingsRow
                title="Which sound to play"
                description="Every option is soft and short, so none of them will startle you."
              >
                <Dropdown
                  value={liveNotifications.sound_type || 'boop'}
                  onChange={(v) => updateLiveNotifications({ sound_type: v })}
                  className="w-full"
                  ariaLabel="Notification sound"
                  options={[
                    { value: 'boop', label: 'Subtle Boop (Default)' },
                    { value: 'tick', label: 'Cozy Knock' },
                    { value: 'soft', label: 'Fireplace Crackle' },
                    { value: 'whisper', label: 'Raindrop' },
                    { value: 'gentle', label: 'Wind Chime' },
                  ]}
                />
              </SettingsRow>
            )}

            <SettingsRow
              title="Send a test"
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
                  style={testFlash ? { backgroundColor: 'color-mix(in srgb, var(--color-success) 50%, transparent)', transition: 'none' } : undefined}
                >
                  <Bell size={16} />
                  Test
                </button>
              }
            />
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
