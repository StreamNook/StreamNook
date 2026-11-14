// Settings > Automations: whether rules run, this device's away, the pace and
// reach every rule shares, then the rules, Try it and Activity. Rust owns the
// rules and every write to them (the automations commands); the preferences
// are ordinary settings, written field by field. Phones show the same page.
import { useEffect, useState } from 'react';
import { useAppStore } from '../../stores/AppStore';
import { SegmentedSelect, SettingsRow, SettingsSection } from './_primitives';
import { InlineSlider, SubControl, SubControls } from '../plugins/settingsPageKit';
import { Toggle } from '../ui/Toggle';
import { EntryInput } from './entryInput';
import { useSettingReset } from './settingReset';
import { IS_MAC, IS_MOBILE } from '../../utils/platform';
import { setSettingsFields } from '../../utils/settingsFields';
import { Logger } from '../../utils/logger';
import { refreshAutomationStatus, useAutomationStatusStore, useNowMs } from '../../hooks/useAutomationStatus';
import {
  DEFAULT_AWAY_MESSAGE,
  automationsApi,
  awayMessageOr,
  errorText,
  secondsText,
  spanText,
} from '../../utils/automations';
import type { AutomationRunState, CloseToTrayMode } from '../../types';
import RuleList from './automations/RuleList';
import TryIt from './automations/TryIt';
import ActivityLog from './automations/ActivityLog';
import { RemindersGroup } from './automations/RemindersGroup';
import { FIELD, INPUT, LIGHT_WARN, PRIMARY_BUTTON, QUIET_BUTTON, RunDot } from './automations/parts';

const RUN_OPTIONS: { value: AutomationRunState; label: string }[] = [
  { value: 'off', label: 'Off' },
  { value: 'test', label: 'Test' },
  { value: 'on', label: 'On' },
];

const RUN_WORDS: Record<AutomationRunState, string> = {
  off: 'Nothing runs. Your rules keep their switches for when you turn this on.',
  test: 'Rules write what they would do to Activity and send nothing.',
  on: 'Rules set to Live act for you. Rules in Test still only write to Activity.',
};

/** Where rules can run, said once for this platform. */
function whereRulesRun(closeToTray: CloseToTrayMode): string {
  if (IS_MOBILE) return 'Rules run while StreamNook is open on this phone.';
  if (closeToTray === 'always') {
    return IS_MAC ? 'Rules keep running from the menu bar.' : 'Rules keep running from the tray.';
  }
  return 'Rules run while StreamNook is open.';
}

const AutomationsSettings = () => {
  const status = useAutomationStatusStore((s) => s.status);
  const prefs = useAppStore((s) => s.settings.automation_prefs);
  const closeToTray = useAppStore((s) => s.settings.close_to_tray ?? 'with-popouts');
  const initialRow = useAppStore((s) => s.settingsInitialRow);
  const resetFor = useSettingReset();
  const now = useNowMs();
  const [awayText, setAwayText] = useState('');
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    void refreshAutomationStatus();
  }, []);

  const run = status?.run ?? 'off';
  const away = status?.away ?? null;
  const lastAway = status?.last_away_message ?? '';
  const spm = prefs?.sends_per_minute ?? 10;
  const other = prefs?.other_channels ?? false;
  const gap = prefs?.other_channel_gap_seconds ?? 30;
  const notes = prefs?.chat_notes ?? true;
  const whisperDay = prefs?.whisper_people_per_day ?? 20;
  // A settings link to one rule (`streamnook://settings/Automations?rule=<id>`)
  // arrives as the row `rule:<id>`.
  const linkedRule = initialRow?.startsWith('rule:') ? initialRow.slice('rule:'.length) : null;

  const write = (field: string, value: unknown) => {
    setSettingsFields({ [`automation_prefs.${field}`]: value }).catch((err) =>
      Logger.warn('[Automations] preference write failed:', err),
    );
  };
  const act = (work: Promise<unknown>) => {
    work
      .then(() => {
        setProblem(null);
        void refreshAutomationStatus();
      })
      .catch((e: unknown) => setProblem(errorText(e)));
  };
  const setAway = () => {
    act(automationsApi.setAway(awayMessageOr(awayText || lastAway)));
    setAwayText('');
  };
  const saveDeviceName = async (text: string): Promise<string | undefined> => {
    try {
      const stored = await automationsApi.setDeviceName(text);
      void refreshAutomationStatus();
      return stored;
    } catch {
      return undefined;
    }
  };

  const waiting = status?.outage
    ? "Can't reach StreamNook right now; rules wait."
    : status?.busy
      ? 'StreamNook is busy; rules wait a moment.'
      : null;
  const kept = status && run !== 'off' && status.chats_kept.length > 0 ? status.chats_kept.length : 0;

  return (
    <div className={IS_MOBILE ? 'space-y-6' : 'space-y-8'}>
      <SettingsSection
        label="Automations"
        description={`${whereRulesRun(closeToTray)} If StreamNook is open on more than one device, the first one to see something answers it.`}
      >
        <SettingsRow
          title="Status"
          titleBadge={<RunDot run={run} />}
          description={RUN_WORDS[run]}
          disabled={!status}
          control={
            <SegmentedSelect value={run} options={RUN_OPTIONS} onChange={(state) => act(automationsApi.setRun(state))} />
          }
        >
          {(waiting || kept > 0 || problem) && (
            <div className="space-y-1">
              {waiting && <p className={`text-[12px] leading-relaxed text-warning ${LIGHT_WARN}`}>{waiting}</p>}
              {kept > 0 && (
                <p className="text-[12px] leading-relaxed text-textMuted">
                  Rules keep {kept} {kept === 1 ? 'chat' : 'chats'} open to listen in.
                </p>
              )}
              {problem && (
                <p role="alert" className="text-[12px] leading-relaxed text-error">
                  {problem}
                </p>
              )}
            </div>
          )}
        </SettingsRow>
        <SettingsRow
          title="Away"
          description={
            away
              ? `Away for ${spanText(away.since_ms, now)}: ${away.message}`
              : 'Rules set to answer only while you are away use this.'
          }
          help="Away is set on this device. /afk and /back do the same where you type them."
          disabled={!status}
          control={
            away ? (
              <button type="button" onClick={() => act(automationsApi.setAway(null))} className={QUIET_BUTTON}>
                Clear
              </button>
            ) : (
              <button type="button" onClick={setAway} className={PRIMARY_BUTTON}>
                Set away
              </button>
            )
          }
        >
          {!away && (
            <label className="block">
              <span className="mb-1 block text-[11px] text-textMuted">Message</span>
              <input
                value={awayText}
                maxLength={200}
                onChange={(e) => setAwayText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') setAway();
                }}
                placeholder={lastAway || DEFAULT_AWAY_MESSAGE}
                className={INPUT}
              />
            </label>
          )}
        </SettingsRow>
        <SettingsRow
          title="Pace"
          description="The most messages your rules send in a minute, across all your devices."
          onReset={resetFor('automation_prefs.sends_per_minute')}
          control={
            <InlineSlider
              value={spm}
              min={1}
              max={20}
              label="Messages a minute"
              format={(v) => `${v}/min`}
              onChange={(v) => write('sends_per_minute', v)}
            />
          }
        />
        <SettingsRow
          title="Other channels"
          description="Rules may post in chats that aren't yours, at most once per chat in each gap."
          onReset={resetFor('automation_prefs.other_channels')}
          control={<Toggle enabled={other} onChange={() => write('other_channels', !other)} ariaLabel="Other channels" />}
        >
          {other && (
            <SubControls>
              <SubControl
                title="Gap"
                onReset={resetFor('automation_prefs.other_channel_gap_seconds')}
                control={
                  <InlineSlider
                    value={gap}
                    min={10}
                    max={600}
                    step={10}
                    label="Gap"
                    format={secondsText}
                    onChange={(v) => write('other_channel_gap_seconds', v)}
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>
        <SettingsRow
          title="Chat notes"
          description="A line only you see in chat when a rule acts, or would act in Test."
          onReset={resetFor('automation_prefs.chat_notes')}
          control={<Toggle enabled={notes} onChange={() => write('chat_notes', !notes)} ariaLabel="Chat notes" />}
        />
        <SettingsRow
          title="Whisper people a day"
          description="The most new people your rules may whisper in a day, across all your devices."
          onReset={resetFor('automation_prefs.whisper_people_per_day')}
          control={
            <InlineSlider
              value={whisperDay}
              min={1}
              max={30}
              label="People a day"
              format={(v) => `${v}/day`}
              onChange={(v) => write('whisper_people_per_day', v)}
            />
          }
        />
        <SettingsRow
          title="Device name"
          description="Shown on your other devices when this one answers first."
          disabled={!status}
          control={
            <EntryInput
              value={status?.device_name ?? ''}
              save={saveDeviceName}
              maxLength={40}
              aria-label="Device name"
              spellCheck={false}
              className={`${FIELD} ${IS_MOBILE ? 'w-40' : 'w-48'}`}
            />
          }
        />
      </SettingsSection>

      <SettingsSection
        label="Rules"
        description="Each rule waits for something in chat, checks its conditions, then acts. New rules start in Test."
        bare
      >
        <RuleList
          key={linkedRule ?? 'rules'}
          status={status}
          now={now}
          initialOpen={linkedRule}
          otherChannelsOn={other}
        />
      </SettingsSection>

      <SettingsSection
        label="Reminders"
        id="automation-reminders"
        description="Reminders follow their own switches, not the status above. Make one into a rule to give it conditions, Test and Activity."
        bare
      >
        <RemindersGroup status={status} now={now} />
      </SettingsSection>

      <SettingsSection
        label="Try It"
        description="See which rules would act on a message or an event, and what they would post. Nothing is sent."
        bare
      >
        <TryIt status={status} />
      </SettingsSection>

      <SettingsSection label="Activity" description="What your rules did, would have done in Test, and skipped." bare>
        <ActivityLog now={now} />
      </SettingsSection>
    </div>
  );
};

export default AutomationsSettings;
