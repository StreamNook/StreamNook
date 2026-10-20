// The setup wizard's Notifications step on the phone: one switch per alert
// type Rust offers in setup (`get_alert_types`, the ones marked `setup`), all
// off until chosen. Turning one on writes its setting and only then asks
// Android for the notification permission; a refusal is said plainly, with the
// way to Android's settings. The iPhone draws the same step (SetupWizard.swift).
import React, { createElement, useEffect, useState } from 'react';
import { ArrowSquareOut } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { GROUP_ICON, GROUP_ICON_STROKE, GroupCard, GroupIcon } from '../ui/GroupedList';
import { settingIcon } from '../screens/settingsIcons';
import { applyDelivery, ensureNotificationPermission, getNotifyDelivery, type NotifyDelivery } from '../notifications';
import { openNotificationSettings } from '../nativeBridge';
import { setSettingsFields } from '../../utils/settingsFields';
import { Logger } from '../../utils/logger';
import type { AlertType } from '../alertTypes';

interface Props {
  types: AlertType[];
  choices: Record<string, boolean>;
  onChoose: (id: string, on: boolean) => void;
}

const AWAY_TITLE = "Mentions while you're away";
const AWAY_DETAIL = "StreamNook's server watches chat in channels you follow for your name and replies, and keeps matches for 7 days.";

const NotificationsStep: React.FC<Props> = ({ types, choices, onChoose }) => {
  const [delivery, setDelivery] = useState<NotifyDelivery | null>(null);
  const awayOn = useAppStore((s) => s.settings.mentions?.away_alerts === true);
  // Bumped after a prompt, and read again on return from Android's settings.
  const [readToken, setReadToken] = useState(0);

  useEffect(() => {
    let cancelled = false;
    const read = async () => {
      const next = await getNotifyDelivery();
      if (!cancelled) setDelivery(next);
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

  const choose = async (type: AlertType, on: boolean) => {
    onChoose(type.id, on);
    // Choosing one also turns the master switch on, and keeps the system shade
    // as the delivery surface, as the Notifications page does.
    const changes: Record<string, unknown> = { [type.setting]: on };
    if (on) {
      changes['live_notifications.enabled'] = true;
      changes['live_notifications.use_native_notifications'] = true;
    }
    try {
      await setSettingsFields(changes);
    } catch (err) {
      Logger.warn('[NotificationsStep] save failed:', err);
    }
    if (!on) return;
    const current = useAppStore.getState().settings.live_notifications;
    applyDelivery({ ...current, enabled: true, [type.setting.split('.')[1]]: true });
    if (delivery?.permission !== 'granted') {
      await ensureNotificationPermission();
      setReadToken((n) => n + 1);
    }
  };

  // Off until chosen. Its pushes follow the Mentions and replies alert, so
  // the permission is asked only when that one is on.
  const chooseAway = async (on: boolean) => {
    try {
      await setSettingsFields({ 'mentions.away_alerts': on });
    } catch (err) {
      Logger.warn('[NotificationsStep] save failed:', err);
    }
  };

  const anyChosen = types.some((t) => choices[t.id]);
  const permission = delivery?.permission;
  const refused =
    anyChosen && (permission === 'denied' || permission === 'blocked' || (permission === 'granted' && delivery?.appEnabled === false));

  return (
    <div className="w-full max-w-md text-left">
      <GroupCard>
        {types.map((type) => {
          const on = choices[type.id] === true;
          return (
            <button
              key={type.id}
              type="button"
              role="switch"
              aria-checked={on}
              onClick={() => void choose(type, !on)}
              className="sn-touch w-full flex items-start gap-3 py-2.5 text-left active:opacity-70"
            >
              {/* Glyph and switch sit on the title's line, as the settings rows place them. */}
              <GroupIcon>
                {createElement(settingIcon('Notify me about', type.title), {
                  size: GROUP_ICON,
                  strokeWidth: GROUP_ICON_STROKE,
                  className: 'text-textSecondary',
                })}
              </GroupIcon>
              <span className="flex-1 min-w-0">
                <span className="block text-[15px] leading-6 text-textPrimary">{type.title}</span>
                <span className="block text-[13px] leading-[18px] text-textSecondary text-pretty">{type.detail}</span>
              </span>
              <span
                aria-hidden="true"
                className={`shrink-0 w-11 h-6 rounded-full relative transition-colors ${on ? 'bg-accent' : 'bg-surface'}`}
              >
                <span
                  className="absolute top-0.5 w-5 h-5 rounded-full bg-white transition-[left] duration-200 ease-out"
                  style={{ left: on ? 22 : 2 }}
                />
              </span>
            </button>
          );
        })}
      </GroupCard>

      {/* Mentions while you're away: its own card, since it is a server
          watching for you rather than an alert type. */}
      <div className="mt-3">
        <GroupCard>
          <button
            type="button"
            role="switch"
            aria-checked={awayOn}
            onClick={() => void chooseAway(!awayOn)}
            className="sn-touch w-full flex items-start gap-3 py-2.5 text-left active:opacity-70"
          >
            <GroupIcon>
              {createElement(settingIcon('Mentions', AWAY_TITLE), {
                size: GROUP_ICON,
                strokeWidth: GROUP_ICON_STROKE,
                className: 'text-textSecondary',
              })}
            </GroupIcon>
            <span className="flex-1 min-w-0">
              <span className="block text-[15px] leading-6 text-textPrimary">{AWAY_TITLE}</span>
              <span className="block text-[13px] leading-[18px] text-textSecondary text-pretty">{AWAY_DETAIL}</span>
            </span>
            <span
              aria-hidden="true"
              className={`shrink-0 w-11 h-6 rounded-full relative transition-colors ${awayOn ? 'bg-accent' : 'bg-surface'}`}
            >
              <span
                className="absolute top-0.5 w-5 h-5 rounded-full bg-white transition-[left] duration-200 ease-out"
                style={{ left: awayOn ? 22 : 2 }}
              />
            </span>
          </button>
        </GroupCard>
      </div>

      {/* Its room is kept while hidden: the page is centered, so a line
          appearing under the card would otherwise shift the switches. */}
      <div
        aria-hidden={!refused}
        className={`mt-4 flex flex-col items-center gap-3 text-center ${refused ? '' : 'invisible'}`}
      >
        <p className="px-2 text-sm text-textSecondary text-balance">
          Notifications are turned off for StreamNook in Android settings, so these alerts will not arrive until you
          allow them there.
        </p>
        <button
          type="button"
          tabIndex={refused ? 0 : -1}
          onClick={openNotificationSettings}
          className="glass-button sn-touch relative px-5 text-[13.5px] font-medium text-textPrimary flex items-center justify-center gap-1.5 after:absolute after:-inset-y-3 after:inset-x-0"
        >
          Open Android settings
          <ArrowSquareOut size={14} />
        </button>
      </div>
    </div>
  );
};

export default NotificationsStep;
