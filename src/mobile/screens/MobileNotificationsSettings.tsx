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
import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowSquareOut, BatteryCharging, BellSimple, CaretRight, Warning } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
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

const TOGGLES: {
  key: keyof LiveNotificationSettings;
  channel: NotifyChannelId;
  label: string;
  description: string;
}[] = [
  {
    key: 'show_live_notifications',
    channel: NOTIFY_CHANNEL.live,
    label: 'Channels going live',
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

const SectionLabel: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide mb-1.5 px-1">
    {children}
  </div>
);

const MobileNotificationsSettings: React.FC = () => {
  const settings = useAppStore((s) => s.settings);
  const updateSettings = useAppStore((s) => s.updateSettings);
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

  const patch = useCallback(
    async (changes: Partial<LiveNotificationSettings>) => {
      const current = settings.live_notifications;
      if (!current) return;
      const next = {
        ...current,
        ...changes,
        // On mobile the system shade IS the delivery surface, so keep the
        // native flag on whenever any category is enabled.
        use_native_notifications: true,
      };
      await updateSettings({ ...settings, live_notifications: next });
      syncBackgroundChecks(next);
      // Push registration follows the same switches: registered while the
      // master and push toggles are both on, dropped from the server otherwise.
      if (next.enabled !== false && next.push_notifications !== false) {
        void invoke('push_register').catch(() => {});
      } else {
        void invoke('push_unregister').catch(() => {});
      }
    },
    [settings, updateSettings],
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

  return (
    <div className="flex flex-col gap-4">
      {/* Master switch. Off means nothing arrives by any route, so it says so
          rather than leaving the categories below looking live. */}
      <div className="glass-panel p-3.5 flex items-center gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <BellSimple size={17} className="text-accent" />
            <span className="text-[14.5px] font-semibold text-textPrimary">Notifications</span>
          </div>
          <div className="text-[12px] text-textMuted leading-snug mt-1">
            {masterOn
              ? 'StreamNook can alert you about the categories below.'
              : 'All StreamNook notifications are off.'}
          </div>
        </div>
        <Toggle on={masterOn} onChange={() => void patch({ enabled: !masterOn })} />
      </div>

      {masterOn && (
        <>
          {/* Permission. Whenever it is missing, BOTH routes are offered: the
              in-app prompt stops working once Android decides the refusal is
              permanent, and nothing in the app can tell that apart reliably, so
              the settings route is never hidden behind a guess. */}
          {!granted || appOff ? (
            <div className="glass-panel p-3.5">
              <div className="flex items-center gap-2 mb-1.5">
                <Warning size={17} className="text-warning" />
                <span className="text-[14.5px] font-semibold text-textPrimary">
                  {appOff ? 'Turned off in Android settings' : 'Notifications are not allowed yet'}
                </span>
              </div>
              <p className="text-[12.5px] text-textSecondary leading-relaxed mb-2.5">
                {appOff
                  ? 'StreamNook has permission, but notifications are switched off for the app in Android settings. Nothing will arrive until that is turned back on.'
                  : permission === 'blocked'
                    ? 'Android will not ask again, so this has to be turned on in system settings.'
                    : 'Allow notifications so alerts reach you when StreamNook is closed.'}
              </p>
              <div className="flex flex-col gap-2">
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
              </div>
            </div>
          ) : (
            <div className="glass-panel p-3.5">
              <div className="flex items-center gap-2 mb-1.5">
                <BellSimple size={17} className="text-accent" />
                <span className="text-[14.5px] font-semibold text-textPrimary">
                  System notifications
                </span>
              </div>
              <p className="text-[12.5px] text-textSecondary leading-relaxed mb-2.5">
                StreamNook can post to your notification shade.
              </p>
              <button
                onClick={openNotificationSettings}
                className="glass-button sn-touch w-full text-[13.5px] font-medium text-textSecondary flex items-center justify-center gap-1.5"
              >
                Sounds and categories in Android
                <ArrowSquareOut size={14} />
              </button>
            </div>
          )}

          <div>
            <SectionLabel>Notify me about</SectionLabel>
            <div className="glass-panel divide-y divide-borderSubtle">
              {TOGGLES.map(({ key, channel, label, description }) => {
                const on = prefs[key] !== false;
                // Silenced in Android's own settings. The in-app toggle cannot
                // undo it, so saying "on" here would be a lie.
                const osBlocked = blocked.includes(channel);
                return (
                  <div key={key} className="flex items-center gap-3 p-3.5">
                    <div className="flex-1 min-w-0">
                      <div className="text-[14px] text-textPrimary">{label}</div>
                      <div className="text-[12px] text-textMuted leading-snug mt-0.5">
                        {description}
                      </div>
                      {osBlocked && (
                        <button
                          onClick={() => openChannelSettings(channel)}
                          className="text-[12px] text-warning mt-1 flex items-center gap-1"
                        >
                          Silenced in Android settings
                          <ArrowSquareOut size={12} />
                        </button>
                      )}
                    </div>
                    <Toggle
                      on={on && !osBlocked}
                      disabled={osBlocked}
                      onChange={() => void patch({ [key]: !on } as Partial<LiveNotificationSettings>)}
                    />
                  </div>
                );
              })}

              {/* Only meaningful while live alerts are on, but shown either way
                  so the setting does not appear and disappear under the reader. */}
              <button
                onClick={() => setChannelsOpen(true)}
                className="w-full flex items-center gap-3 p-3.5 text-left"
              >
                <div className="flex-1 min-w-0">
                  <div className="text-[14px] text-textPrimary">Which channels</div>
                  <div className="text-[12px] text-textMuted leading-snug mt-0.5">
                    {mutedCount > 0
                      ? `Everything you follow except ${mutedCount}`
                      : 'Everything you follow'}
                  </div>
                </div>
                <CaretRight size={16} className="text-textMuted shrink-0" />
              </button>
            </div>
          </div>

          <div>
            <SectionLabel>Background checking</SectionLabel>
            <div className="glass-panel divide-y divide-borderSubtle">
              {/* Battery first, deliberately. Android withholds network from
                  background work entirely once an app it considers unused drops
                  far enough down, so this is not a tuning knob: it decides
                  whether closed-app notifications happen at all. */}
              {batteryExempt === false && (
                <div className="p-3.5">
                  <div className="flex items-center gap-2 mb-1">
                    <BatteryCharging size={17} className="text-warning" />
                    <span className="text-[14px] font-semibold text-textPrimary">
                      Alerts will be unreliable
                    </span>
                  </div>
                  <p className="text-[12px] text-textMuted leading-snug mb-2.5">
                    Android is limiting StreamNook in the background. Until that is lifted, alerts
                    can be delayed by hours or not arrive at all while the app is closed.
                  </p>
                  <button
                    onClick={requestIgnoreBatteryOptimizations}
                    className="glass-button sn-touch w-full text-[13.5px] font-semibold text-textPrimary"
                  >
                    Let StreamNook run in the background
                  </button>
                </div>
              )}

              <div className="flex items-center gap-3 p-3.5">
                <div className="flex-1 min-w-0">
                  <div className="text-[14px] text-textPrimary">Instant alerts</div>
                  <div className="text-[12px] text-textMuted leading-snug mt-0.5">
                    Pushed the moment a channel goes live. When off, alerts arrive on the
                    scheduled check below.
                  </div>
                </div>
                <Toggle
                  on={prefs.push_notifications !== false}
                  onChange={() =>
                    void patch({ push_notifications: prefs.push_notifications === false })
                  }
                />
              </div>

              {/* No separate background toggle: the periodic check IS the
                  delivery pipeline, app open or closed, so the master switch
                  above is the only honest control. A second toggle here meant
                  "notifications on" could silently deliver nothing. */}
              <div className="p-3.5">
                <div className="text-[14px] text-textPrimary mb-0.5">How often to check</div>
                <div className="text-[12px] text-textMuted leading-snug mb-2.5">
                  Checking less often uses less battery. Android may still wait longer than this.
                </div>
                <div className="flex gap-2">
                  {INTERVALS.map((m) => {
                    const active = (prefs.background_interval_minutes ?? 15) === m;
                    return (
                      <button
                        key={m}
                        onClick={() => void patch({ background_interval_minutes: m })}
                        className={`flex-1 sn-touch rounded-lg py-2 text-[13px] transition-colors ${
                          active
                            ? 'bg-accent text-white font-semibold'
                            : 'bg-surface text-textSecondary'
                        }`}
                      >
                        {m} min
                      </button>
                    );
                  })}
                </div>
              </div>
            </div>
          </div>
        </>
      )}

      <button
        onClick={() =>
          void postSystemNotification({
            title: 'StreamNook',
            body: 'Notifications are working.',
          })
        }
        className="glass-button sn-touch w-full text-[13.5px] font-medium text-textSecondary"
      >
        Send a test notification
      </button>

      <NotifiedChannelsSheet open={channelsOpen} onClose={() => setChannelsOpen(false)} />
    </div>
  );
};

export default MobileNotificationsSettings;
