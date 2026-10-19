// Mobile notification settings.
//
// The desktop panel is built around in-window delivery (toasts, the Dynamic
// Island, positioning and edge offsets), none of which exist on a phone: the
// system shade is the only surface that works when the app is backgrounded.
// So this is its own panel over the SAME persisted settings fields.
//
// Three things silence notifications and only one of them is the runtime
// permission. The app can be switched off wholesale in system settings, and a
// single category can sit at IMPORTANCE_NONE, and the notification plugin can
// see neither. A panel that reports only the permission is how someone ends up
// being told everything is fine while they receive nothing, so all three are
// read here and each one names its own fix.
import React, { createElement, useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowSquareOut } from 'phosphor-react';
import { BatteryWarning, BellRing, ChevronRight, Send, TriangleAlert } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { SettingsRow, SettingsSection } from '../../components/settings/_primitives';
import { thenRun, useSettingReset } from '../../components/settings/settingReset';
import { GROUP_ICON, GROUP_ICON_STROKE, GroupCard, GroupIcon, GroupRow } from '../ui/GroupedList';
import { settingIcon } from './settingsIcons';
import { NotifiedChannelsSheet } from './NotifiedChannelsSheet';
import {
  ensureNotificationPermission,
  getNotifyDelivery,
  postSystemNotification,
  syncBackgroundChecks,
  NOTIFY_CHANNEL,
  type NotifyChannelId,
  type NotifyDelivery,
} from '../notifications';
import {
  isIgnoringBatteryOptimizations,
  openChannelSettings,
  openNotificationSettings,
  requestIgnoreBatteryOptimizations,
} from '../nativeBridge';
import type { LiveNotificationSettings } from '../../types';
import { Logger } from '../../utils/logger';
import { fieldsIn, setSettingsFields } from '../../utils/settingsFields';

const TOGGLES: {
  key: keyof LiveNotificationSettings;
  channel: NotifyChannelId;
  label: string;
  description: string;
}[] = [
  {
    key: 'show_live_notifications',
    channel: NOTIFY_CHANNEL.live,
    label: 'Live channels',
    description: 'Alert when a channel you follow starts streaming',
  },
  {
    key: 'show_drops_notifications',
    channel: NOTIFY_CHANNEL.drops,
    label: 'Drops',
    description: 'Campaign progress and claimable rewards',
  },
  {
    key: 'show_badge_notifications',
    channel: NOTIFY_CHANNEL.badges,
    label: 'New badges',
    description: 'When a new global badge becomes available',
  },
  // No Whispers row. Whispers are not implemented on this platform at all, so
  // the toggle offered control over something that could never happen. The
  // setting itself still exists and desktop still uses it; it just is not
  // advertised here.
  //
  // No Channel points row either. The composer's "+N" float covers the app
  // being open, and a backgrounded WebView cannot poll, so the only state the
  // old notification could actually fire in was picture-in-picture, where it
  // flooded the shade. The OS channel id stays reserved in case the Rust watch
  // heartbeat ever reports credits itself.
];

const INTERVALS = [15, 30, 60];

const Toggle: React.FC<{ on: boolean; disabled?: boolean; onChange: () => void }> = ({
  on,
  disabled,
  onChange,
}) => (
  <button
    onClick={onChange}
    disabled={disabled}
    role="switch"
    aria-checked={on}
    className={`shrink-0 w-11 h-6 rounded-full relative transition-colors ${
      on ? 'bg-accent' : 'bg-surface'
    } ${disabled ? 'opacity-40' : ''}`}
  >
    <span
      className="absolute top-0.5 w-5 h-5 rounded-full bg-white transition-[left] duration-200 ease-out"
      style={{ left: on ? 22 : 2 }}
    />
  </button>
);

/** What follows a change to the switches: the background checks, and the push
 *  registration, held while the master and push toggles are both on and
 *  dropped from the server otherwise. */
function applyDelivery(next: Partial<LiveNotificationSettings>): void {
  syncBackgroundChecks(next);
  if (next.enabled !== false && next.push_notifications !== false) {
    void invoke('push_register').catch(() => {});
  } else {
    void invoke('push_unregister').catch(() => {});
  }
}

const MobileNotificationsSettings: React.FC = () => {
  const settings = useAppStore((s) => s.settings);
  const resetFor = useSettingReset();
  const [delivery, setDelivery] = useState<NotifyDelivery | null>(null);
  const [batteryExempt, setBatteryExempt] = useState<boolean | null>(null);
  const [channelsOpen, setChannelsOpen] = useState(false);
  const mutedCount = settings.live_notifications?.muted_live_channels?.length ?? 0;

  // Every fix on this screen happens in Android's own settings, which means
  // leaving the app and coming back. Without re-reading on return the panel
  // keeps showing the problem that was just fixed, which reads as the fix
  // having failed. The bump is what event handlers use to ask for a re-read;
  // the read itself lives inside the effect so its state commits land after a
  // real await rather than synchronously in the effect body.
  const [readToken, setReadToken] = useState(0);
  const refresh = useCallback(() => setReadToken((n) => n + 1), []);

  useEffect(() => {
    let cancelled = false;
    const read = async () => {
      const next = await getNotifyDelivery();
      if (cancelled) return;
      setDelivery(next);
      setBatteryExempt(isIgnoringBatteryOptimizations());
    };
    void read();
    const onVisible = () => {
      if (document.visibilityState === 'visible') void read();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      cancelled = true;
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, [readToken]);

  const prefs: Partial<LiveNotificationSettings> = settings.live_notifications ?? {};
  const masterOn = prefs.enabled !== false;

  // One field per write: the changed switches only, never the whole group from
  // this screen's copy, which could put back a list another surface had just
  // edited (the muted channels, from the player's bell).
  const patch = useCallback(
    async (changes: Partial<LiveNotificationSettings>) => {
      const current = useAppStore.getState().settings.live_notifications;
      if (!current) return;
      // On mobile the system shade IS the delivery surface, so keep the
      // native flag on whenever any category is enabled.
      const written = { ...changes, use_native_notifications: true };
      try {
        await setSettingsFields(fieldsIn('live_notifications', written));
      } catch (err) {
        Logger.warn('[MobileNotificationsSettings] save failed:', err);
        useAppStore.getState().addToast('Could not save that change.', 'error');
        return;
      }
      applyDelivery({ ...current, ...written });
    },
    [],
  );

  const requestPermission = async () => {
    await ensureNotificationPermission();
    refresh();
  };

  const permission = delivery?.permission ?? 'default';
  const granted = permission === 'granted';
  // Granted but switched off at the app level: the runtime permission alone
  // would report this as working.
  const appOff = granted && delivery?.appEnabled === false;
  const blocked = delivery?.blockedChannels ?? [];

  // A reset goes through the same follow-up as a toggle: the background
  // checks and the push registration track the switches.
  const resetWith = (...specs: Parameters<typeof resetFor>) =>
    thenRun(resetFor(...specs), (next) => {
      if (next.live_notifications) applyDelivery(next.live_notifications);
    });

  // Sections in the settings pages' grouped style, named as the settings
  // catalog names them so a search hit lands on its row.
  return (
    <div className="space-y-6">
      {/* Master switch. Off means nothing arrives by any route, so it says so
          rather than leaving the categories below looking live. */}
      <SettingsSection label="Notifications">
        <SettingsRow
          title="Notifications"
          description={
            masterOn ? 'StreamNook can alert you about the categories below.' : 'All StreamNook notifications are off.'
          }
          onReset={resetWith(['live_notifications.enabled', true])}
          control={<Toggle on={masterOn} onChange={() => void patch({ enabled: !masterOn })} />}
        />
      </SettingsSection>

      {masterOn && (
        <>
          {/* Permission. Whenever it is missing, BOTH routes are offered: the
              in-app prompt stops working once Android decides the refusal is
              permanent, and nothing in the app can tell that apart reliably, so
              the settings route is never hidden behind a guess. A state, not a
              setting, so a card with no label of its own. */}
          {!granted || appOff ? (
            <GroupCard>
              <StatusBlock
                icon={<TriangleAlert size={GROUP_ICON} strokeWidth={GROUP_ICON_STROKE} className="text-warning" />}
                title={appOff ? 'Turned off in Android settings' : 'Notifications are not allowed yet'}
                text={
                  appOff
                    ? 'StreamNook has permission, but notifications are switched off for the app in Android settings. Nothing will arrive until that is turned back on.'
                    : permission === 'blocked'
                      ? 'Android will not ask again, so this has to be turned on in system settings.'
                      : 'Allow notifications so alerts reach you when StreamNook is closed.'
                }
              >
                {!granted && permission !== 'blocked' && (
                  <button
                    onClick={() => void requestPermission()}
                    className="glass-button sn-touch w-full text-[13.5px] font-semibold text-textPrimary"
                  >
                    Allow notifications
                  </button>
                )}
                <button
                  onClick={openNotificationSettings}
                  className="glass-button sn-touch w-full text-[13.5px] font-medium text-textSecondary flex items-center justify-center gap-1.5"
                >
                  Open system settings
                  <ArrowSquareOut size={14} />
                </button>
              </StatusBlock>
            </GroupCard>
          ) : (
            <GroupCard>
              <StatusBlock
                icon={<BellRing size={GROUP_ICON} strokeWidth={GROUP_ICON_STROKE} className="text-textSecondary" />}
                title="System notifications"
                text="StreamNook can post to your notification shade."
              >
                <button
                  onClick={openNotificationSettings}
                  className="glass-button sn-touch w-full text-[13.5px] font-medium text-textSecondary flex items-center justify-center gap-1.5"
                >
                  Sounds and categories in Android
                  <ArrowSquareOut size={14} />
                </button>
              </StatusBlock>
            </GroupCard>
          )}

          <SettingsSection label="Notify me about">
            {TOGGLES.map(({ key, channel, label, description }) => {
              const on = prefs[key] !== false;
              // Silenced in Android's own settings. The in-app toggle cannot
              // undo it, so saying "on" here would be a lie.
              const osBlocked = blocked.includes(channel);
              return (
                <SettingsRow
                  key={key}
                  title={label}
                  description={description}
                  onReset={resetWith([`live_notifications.${key}`, true])}
                  control={
                    <Toggle
                      on={on && !osBlocked}
                      disabled={osBlocked}
                      onChange={() => void patch({ [key]: !on } as Partial<LiveNotificationSettings>)}
                    />
                  }
                >
                  {osBlocked && (
                    <button
                      onClick={() => openChannelSettings(channel)}
                      className="-mt-2 text-[13px] text-warning flex items-center gap-1"
                    >
                      Silenced in Android settings
                      <ArrowSquareOut size={12} />
                    </button>
                  )}
                </SettingsRow>
              );
            })}

            {/* Only meaningful while live alerts are on, but shown either way
                so the setting does not appear and disappear under the reader. */}
            <GroupRow
              data-setting-row="Which channels"
              onClick={() => setChannelsOpen(true)}
              icon={createElement(settingIcon('Notify me about', 'Which channels'), {
                size: GROUP_ICON,
                strokeWidth: GROUP_ICON_STROKE,
                className: 'text-textSecondary',
              })}
              detail={mutedCount > 0 ? `Everything you follow except ${mutedCount}` : 'Everything you follow'}
              trailing={
                <span className="flex h-6 shrink-0 items-center self-start">
                  <ChevronRight size={16} className="text-textMuted" />
                </span>
              }
            >
              Which channels
            </GroupRow>
          </SettingsSection>

          <SettingsSection label="Background checking">
            {/* Battery first, deliberately. Android withholds network from
                background work entirely once an app it considers unused drops
                far enough down, so this is not a tuning knob: it decides
                whether closed-app notifications happen at all. */}
            {batteryExempt === false && (
              <StatusBlock
                icon={<BatteryWarning size={GROUP_ICON} strokeWidth={GROUP_ICON_STROKE} className="text-warning" />}
                title="Alerts will be unreliable"
                text="Android is limiting StreamNook in the background. Until that is lifted, alerts can be delayed by hours or not arrive at all while the app is closed."
              >
                <button
                  onClick={requestIgnoreBatteryOptimizations}
                  className="glass-button sn-touch w-full text-[13.5px] font-semibold text-textPrimary"
                >
                  Let StreamNook run in the background
                </button>
              </StatusBlock>
            )}

            <SettingsRow
              title="Instant alerts"
              description="Pushed the moment a channel goes live. When off, alerts arrive on the scheduled check below."
              onReset={resetWith(['live_notifications.push_notifications', true])}
              control={
                <Toggle
                  on={prefs.push_notifications !== false}
                  onChange={() => void patch({ push_notifications: prefs.push_notifications === false })}
                />
              }
            />

            {/* No separate background toggle: the periodic check IS the
                delivery pipeline, app open or closed, so the master switch
                above is the only honest control. A second toggle here meant
                "notifications on" could silently deliver nothing. */}
            <SettingsRow
              title="How often to check"
              description="Checking less often uses less battery. Android may still wait longer than this."
              onReset={resetWith(['live_notifications.background_interval_minutes', 15])}
            >
              <div className="flex gap-2">
                {INTERVALS.map((m) => {
                  const active = (prefs.background_interval_minutes ?? 15) === m;
                  return (
                    <button
                      key={m}
                      onClick={() => void patch({ background_interval_minutes: m })}
                      className={`flex-1 sn-touch rounded-lg py-2 text-[13px] transition-colors ${
                        active ? 'bg-accent text-white font-semibold' : 'bg-surface text-textSecondary'
                      }`}
                    >
                      {m} min
                    </button>
                  );
                })}
              </div>
            </SettingsRow>
          </SettingsSection>
        </>
      )}

      <GroupCard>
        <GroupRow
          onClick={() =>
            void postSystemNotification({
              title: 'StreamNook',
              body: 'Notifications are working.',
            })
          }
          icon={<Send size={GROUP_ICON} strokeWidth={GROUP_ICON_STROKE} className="text-textSecondary" />}
        >
          Send a test notification
        </GroupRow>
      </GroupCard>

      <NotifiedChannelsSheet open={channelsOpen} onClose={() => setChannelsOpen(false)} />
    </div>
  );
};

/** A state the page reports rather than a setting (a missing permission, a
 *  battery limit): its glyph and name on one line, what it means under the
 *  name, and the buttons that fix it under that, in the rows' text column. */
const StatusBlock: React.FC<{
  icon: React.ReactNode;
  title: string;
  text: string;
  children: React.ReactNode;
}> = ({ icon, title, text, children }) => (
  <div className="py-3">
    <div className="flex items-center gap-3">
      <GroupIcon>{icon}</GroupIcon>
      <span className="text-[15px] leading-6 font-semibold text-textPrimary">{title}</span>
    </div>
    <p className="pl-9 mt-0.5 text-[13px] leading-[18px] text-textSecondary">{text}</p>
    <div className="pl-9 mt-3 flex flex-col gap-2">{children}</div>
  </div>
);

export default MobileNotificationsSettings;
