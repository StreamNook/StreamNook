import { useState } from 'react';
import { Dropdown } from '../ui/Dropdown';
import { Tooltip } from '../ui/Tooltip';
import { Toggle } from '../ui/Toggle';
import { useAppStore } from '../../stores/AppStore';
import {
  ToastPosition,
  DEFAULT_TOAST_POSITION,
  DEFAULT_TOAST_EDGE_OFFSET,
  type LiveNotificationSettings,
  type MentionSettings,
} from '../../types';
import { invoke } from '@tauri-apps/api/core';
import { Award, Bell, ExternalLink, Gift, X } from 'lucide-react';
import { ProviderLogo } from '../ProviderLogo';
import { parseKey } from '../../utils/providerKey';
import { toggleMentionChatMuted } from '../../stores/mentionInboxStore';
import { SettingsSection, SettingsRow } from './_primitives';
import { SoundVolume } from './SoundControls';
import { useSettingReset } from './settingReset';
import { InlineSlider, SubControl, SubControls } from '../plugins/settingsPageKit';
import { useSoundOptions } from '../../hooks/useSoundOptions';
import { playTestNotificationSound } from '../../utils/notificationSound';
import { grantAccolade } from '../../services/supabaseService';
import {
  RESTLESS_ACCOLADE_ID,
  NOTIF_CLICK_THRESHOLD,
  bumpTestNotificationClicks,
} from '../../utils/notifAchievement';
import { alertKey, loadAlertTypes, useAlertTypes } from '../../mobile/alertTypes';
import { openSystemNotifySettings, useSystemNotifyPermission } from '../../utils/systemNotifyPermission';

import { Logger } from '../../utils/logger';
import { IS_LINUX, IS_MAC } from '../../utils/platform';

// The "Notify me about" rows are the alert types Rust lists for desktop
// (services/alert_types.rs, `get_alert_types`), in its order and with the
// words both phones use. Read as soon as this panel's code loads, so the page
// draws whole on its first frame rather than growing a section under the
// reader.
void loadAlertTypes();

// The operating system's name for its own notifications.
const SYSTEM_NAME = IS_MAC ? 'macOS' : IS_LINUX ? 'desktop' : 'Windows';
const IS_WINDOWS_DESKTOP = !IS_MAC && !IS_LINUX;

// What each type's notification does on desktop beyond its one-line detail,
// behind the row's help glyph.
const TYPE_HELP: Record<string, string> = {
  live: 'The moment it starts streaming. Clicking it opens the stream.',
  favorite_live: 'Even a favorite you do not follow on Twitch. Clicking it opens the stream.',
  mentions: 'While that chat is not on screen. Clicking it opens the chat at that message.',
  whispers: 'Each new whisper. Clicking it opens the conversation.',
  drops: 'A notification confirms each drop StreamNook claims for you. New drops: at startup, StreamNook checks your favorite categories and tells you when they have new drops to earn.',
  gift_subs: 'Even for a channel you were not watching at the time.',
  twitch_rewards: 'Which badge you earned or which drop reward is waiting, by name, however it was earned.',
  channel_points: IS_WINDOWS_DESKTOP
    ? 'With the channel\'s face and its own name for its points. While StreamNook is in the background, points for watching do not pop up. Each channel keeps one entry in Action Center with what it gave you this session, and a newer notification for a channel replaces its older one. Rewards button adds a button that opens the channel\'s rewards, and Next reward adds a bar toward the cheapest reward you cannot afford yet. Both need System notifications.'
    : 'With the channel\'s face and its own name for its points. While StreamNook is in the background, points for watching do not pop up.',
  predictions: 'While StreamNook is in the background, with its choices and how long betting stays open. Clicking it opens the stream.',
  updates: 'It opens the changelog to what is new. Turning this off leaves the update button in the title bar and Check for updates in the changelog.',
};

// Every type's switch starts on except predictions, as Rust's defaults have
// them; read only when a saved copy lacks the key.
const OFF_BY_DEFAULT = new Set(['predictions']);

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

// Shows a sample system notification ("Try one" lines).
const TryButton = ({ onClick }: { onClick: () => void }) => (
  <button
    onClick={onClick}
    className="glass-button flex items-center gap-2 px-3 py-1.5 rounded-lg text-textPrimary text-xs font-medium"
  >
    <Bell size={14} />
    Send
  </button>
);

const NotificationsSettings = () => {
  const { settings, updateSettings, currentUser, addToast } = useAppStore();
  const resetFor = useSettingReset();
  const alertTypes = useAlertTypes();
  // macOS asks before StreamNook's first system notification; elsewhere this
  // is granted and never read.
  const { permission, ask } = useSystemNotifyPermission();
  // Brief green flash on press, used as the Test button's feedback instead of a
  // text/width swap. See handleTestNotification + the button's className.
  const [testFlash, setTestFlash] = useState(false);

  const liveNotifications: LiveNotificationSettings = settings.live_notifications || {
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
    show_mention_notifications: true,
    show_prediction_notifications: false,
    use_dynamic_island: true,
    use_toast: true,
    toast_position: DEFAULT_TOAST_POSITION,
    toast_edge_offset: DEFAULT_TOAST_EDGE_OFFSET,
  };
  const systemOn = liveNotifications.use_native_notifications ?? false;

  const soundOptions = useSoundOptions({ defaultId: 'boop' });

  const updateLiveNotifications = (updates: Partial<LiveNotificationSettings>) => {
    updateSettings({
      ...settings,
      live_notifications: {
        ...liveNotifications,
        ...updates,
      },
    });
  };

  // System notifications on a Mac need its permission, asked the first time
  // they would be used: when they are turned on, or when a type is turned on
  // while they are. A refusal is shown under the System notifications row.
  const askIfNeeded = () => {
    if (IS_MAC && permission === 'default') void ask();
  };

  // What counts as a mention, and the chats that never notify (Rust's
  // `MentionSettings`; it reads them on save).
  const mentions: MentionSettings = settings.mentions ?? {};
  const updateMentions = (updates: Partial<MentionSettings>) => {
    updateSettings({ ...settings, mentions: { ...mentions, ...updates } });
  };
  const mutedChats = mentions.muted_channels ?? [];
  // A platform mark only where platforms mix in the list.
  const mutedMixed = new Set(mutedChats.map((r) => parseKey(r).provider)).size > 1;

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

  const handleTestSystemNotification = async () => {
    try {
      await invoke('test_system_notification');
    } catch (error) {
      Logger.error('Failed to send test system notification:', error);
    }
  };

  // A sample channel points or prediction notification as the system shows
  // it, with the extras turned on above. Sample numbers; nothing is spent.
  const handleTestPoints = async (kind: 'one' | 'prediction') => {
    try {
      await invoke('test_points_notification', { kind });
    } catch (error) {
      Logger.error('Failed to send test points notification:', error);
      addToast(`Could not show the sample: ${error}`, 'error');
    }
  };

  const handleTestNotification = async () => {
    // Played inside the press itself: a gesture-started sound plays under any
    // webview's audio policy, and it plays even if the sample fails to send.
    playTestNotificationSound(liveNotifications);
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

  // The list is read once a run, started when this code loaded; until it
  // answers (a moment, the first time) the page waits rather than drawing
  // without its types and then pushing everything under them down.
  if (alertTypes === null) return null;

  return (
    <div className="space-y-8">
      <SettingsSection label="Notifications">
        <SettingsRow
          title="Notifications"
          onReset={resetFor(['live_notifications.enabled', true])}
          description={liveNotifications.enabled
            ? 'StreamNook can alert you about the categories below.'
            : 'All StreamNook notifications are off.'}
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
            description={`Where notifications show up: in the Dynamic Island at the top of the window, as toast popups, and as ${SYSTEM_NAME} notifications while you are elsewhere.`}
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

            <SettingsRow
              title="System notifications"
              onReset={resetFor(['live_notifications.use_native_notifications', false])}
              description={`While you are in another app, or StreamNook is minimized or in the tray, notifications also show as ${SYSTEM_NAME} notifications.`}
              help={IS_WINDOWS_DESKTOP
                ? 'Clicking one, or its button, opens StreamNook on what it is about. They stay silent, since StreamNook already plays your sound. While this is on they take the place of the cards in MultiChat.'
                : 'They stay silent, since StreamNook already plays your sound. While this is on they take the place of the cards in MultiChat.'}
              control={
                <Toggle
                  enabled={systemOn}
                  onChange={() => {
                    updateLiveNotifications({ use_native_notifications: !systemOn });
                    if (!systemOn) askIfNeeded();
                  }}
                />
              }
            >
              {systemOn && (
                <>
                  {/* Only a Mac can refuse. Said under the row it stops, with
                      the way to undo it, the same line the setup step shows. */}
                  {IS_MAC && permission === 'denied' && (
                    <div className="mb-3 flex items-center justify-between gap-4 rounded-lg border border-warning/30 bg-warning/5 py-2.5 pl-4 pr-3">
                      <p className="text-[12px] leading-relaxed text-textSecondary">
                        Notifications are turned off for StreamNook in macOS settings, so these will not reach you outside StreamNook until you allow them there.
                      </p>
                      <button
                        type="button"
                        onClick={openSystemNotifySettings}
                        className="glass-button flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-lg px-3 py-1.5 text-xs font-medium text-textPrimary"
                      >
                        Open macOS settings
                        <ExternalLink size={12} />
                      </button>
                    </div>
                  )}
                  <SubControls>
                    <SubControl
                      title="Try one"
                      control={<TryButton onClick={handleTestSystemNotification} />}
                    />
                  </SubControls>
                </>
              )}
            </SettingsRow>
          </SettingsSection>

          <SettingsSection
            label="Notify me about"
            description="Pick which events are worth a notification."
          >
            {alertTypes.map((type) => {
              const key = alertKey(type);
              if (!key) return null;
              const saved = liveNotifications[key];
              const on = typeof saved === 'boolean' ? saved : !OFF_BY_DEFAULT.has(type.id);
              return (
                <SettingsRow
                  key={type.id}
                  title={type.title}
                  onReset={resetFor(type.setting)}
                  description={type.detail}
                  help={TYPE_HELP[type.id]}
                  control={
                    <Toggle
                      enabled={on}
                      onChange={() => {
                        updateLiveNotifications({ [key]: !on } as Partial<LiveNotificationSettings>);
                        if (!on && systemOn) askIfNeeded();
                      }}
                    />
                  }
                >
                  {/* What a type's own notification can carry on desktop,
                      nested under it while it is on. One expression, so a
                      type with nothing nested passes no children at all and
                      its row keeps the plain height. */}
                  {type.id === 'drops' && on ? (
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
                  ) : type.id === 'channel_points' && on && systemOn ? (
                    <SubControls>
                      {IS_WINDOWS_DESKTOP && (
                        <SubControl
                          title="Rewards button"
                          onReset={resetFor(['live_notifications.points_rewards_button', false])}
                          control={
                            <Toggle
                              enabled={liveNotifications.points_rewards_button ?? false}
                              onChange={() => updateLiveNotifications({
                                points_rewards_button: !(liveNotifications.points_rewards_button ?? false)
                              })}
                            />
                          }
                        />
                      )}
                      {IS_WINDOWS_DESKTOP && (
                        <SubControl
                          title="Next reward"
                          onReset={resetFor(['live_notifications.points_next_reward', false])}
                          control={
                            <Toggle
                              enabled={liveNotifications.points_next_reward ?? false}
                              onChange={() => updateLiveNotifications({
                                points_next_reward: !(liveNotifications.points_next_reward ?? false)
                              })}
                            />
                          }
                        />
                      )}
                      <SubControl
                        title="Try one"
                        control={<TryButton onClick={() => handleTestPoints('one')} />}
                      />
                    </SubControls>
                  ) : type.id === 'predictions' && on && systemOn ? (
                    <SubControls>
                      <SubControl
                        title="Try one"
                        control={<TryButton onClick={() => handleTestPoints('prediction')} />}
                      />
                    </SubControls>
                  ) : null}
                </SettingsRow>
              );
            })}
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

      {/* Outside the master switch: what counts as a mention also decides
          what the Dynamic Island's notification history keeps, notifications
          or not. Whether a mention notifies is its row in Notify me about. */}
      <SettingsSection
        label="Mentions"
        description="Everything that mentions or replies to you is kept in the Dynamic Island's notification history."
      >
        <SettingsRow
          title="Counts as a mention"
          description="Your @name always counts. These count too, for notifications and in that history."
        >
          <SubControls>
            <SubControl
              title="Name without @"
              onReset={resetFor(['mentions.match_bare_login', true])}
              control={
                <Toggle
                  enabled={mentions.match_bare_login ?? true}
                  onChange={() => updateMentions({ match_bare_login: !(mentions.match_bare_login ?? true) })}
                />
              }
            />
            <SubControl
              title="Display name"
              onReset={resetFor(['mentions.match_display_name', true])}
              control={
                <Toggle
                  enabled={mentions.match_display_name ?? true}
                  onChange={() => updateMentions({ match_display_name: !(mentions.match_display_name ?? true) })}
                />
              }
            />
            <SubControl
              title="Highlight words"
              onReset={resetFor(['mentions.highlight_words', true])}
              control={
                <Toggle
                  enabled={mentions.highlight_words ?? true}
                  onChange={() => updateMentions({ highlight_words: !(mentions.highlight_words ?? true) })}
                />
              }
            />
          </SubControls>
        </SettingsRow>
        <SettingsRow
          title="Muted chats"
          onReset={resetFor(['mentions.muted_channels', []])}
          description={mutedChats.length === 0
            ? 'None. Nothing said in a chat you mute notifies you or is kept in that history.'
            : 'Nothing said in these chats notifies you or is kept in that history.'}
        >
          {mutedChats.length > 0 && (
            <div className="mt-3 flex flex-wrap gap-1.5">
              {mutedChats.map((room) => {
                const { provider, channel } = parseKey(room);
                return (
                  <span
                    key={room}
                    className="glass-input inline-flex h-7 items-center gap-1.5 rounded-full pl-2.5 pr-1 text-[12.5px] text-textPrimary"
                  >
                    {mutedMixed && <ProviderLogo provider={provider} size={12} />}
                    {channel}
                    <Tooltip content="Unmute" side="top">
                      <button
                        type="button"
                        aria-label={`Unmute ${channel}`}
                        onClick={() => void toggleMentionChatMuted(room)}
                        className="flex h-5 w-5 items-center justify-center rounded-full text-textMuted transition-colors hover:bg-white/10 hover:text-textPrimary"
                      >
                        <X size={12} />
                      </button>
                    </Tooltip>
                  </span>
                );
              })}
            </div>
          )}
        </SettingsRow>
        {/* Rust tells the server and pulls what it found into that history on
            launch (services/mention_watch.rs); the desktop gets no push. */}
        <SettingsRow
          title="Mentions while you're away"
          onReset={resetFor(['mentions.away_alerts', false])}
          description="StreamNook's server watches chat in channels you follow for your name and replies, and keeps matches for 7 days."
          control={
            <Toggle
              enabled={mentions.away_alerts ?? false}
              onChange={() => updateMentions({ away_alerts: !(mentions.away_alerts ?? false) })}
            />
          }
        />
      </SettingsSection>

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
                Notifications can show in the Dynamic Island (the notification center at the top), as toast popups at any corner or edge you like, or both, and as {SYSTEM_NAME} notifications while you are elsewhere. Clicking one takes action: a live notification starts the stream, a whisper opens the conversation, and an update takes you to the Updates page.
              </p>
            </div>
          </div>
        </div>
      </SettingsSection>
    </div>
  );
};

export default NotificationsSettings;
