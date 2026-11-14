// Settings > Automations. One raised card says whether rules run and holds this
// device's away; then the reminders, the rules, what they did (with Try a
// message), and the limits every rule shares, folded behind their summary. A
// rule opens its editor in place of the page. Rust owns the rules and every
// write to them (the automations commands); the limits are ordinary settings,
// written field by field. Phones show the same page.
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { ChevronDown, Coffee, Plus, SlidersHorizontal } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { SETTINGS_REVEAL_EVENT, SegmentedSelect, SettingsRow, SettingsSection, sectionIdFromLabel } from './_primitives';
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
  accountRef,
  automationsApi,
  awayMessageOr,
  blankRule,
  duplicateRule,
  errorText,
  secondsText,
  spanText,
} from '../../utils/automations';
import type { AutomationRule, AutomationRunState, CloseToTrayMode } from '../../types';
import { GroupCard } from '../../mobile/ui/GroupedList';
import RuleList, { RuleHeaderActions, TemplatePicker } from './automations/RuleList';
import ActivityCard from './automations/ActivityLog';
import { RemindersGroup } from './automations/RemindersGroup';
import { RuleEditor } from './automations/RuleEditor';
import {
  ErrorLine,
  FIELD,
  GHOST_BUTTON,
  HINT,
  INPUT,
  Note,
  QUIET_BUTTON,
  RAISED_CARD,
  RunDot,
  WarnLine,
} from './automations/parts';

const RUN_OPTIONS: { value: AutomationRunState; label: string }[] = [
  { value: 'off', label: 'Off' },
  { value: 'test', label: 'Test' },
  { value: 'on', label: 'On' },
];

const RUN_TITLES: Record<AutomationRunState, string> = {
  off: 'Automations are off',
  test: 'Automations are in Test',
  on: 'Automations are on',
};

const RUN_WORDS: Record<AutomationRunState, string> = {
  off: 'Nothing runs. Your rules keep their switches for when you turn this on.',
  test: 'Rules write what they would do to Activity and send nothing.',
  on: 'Rules set to Live act for you. Rules in Test only write to Activity.',
};

/** The mark column of the status card's rows; the text after it lines up. */
const LEAD = IS_MOBILE ? 'w-6' : 'w-[18px]';
/** What sits under a row's text on a phone starts where the text does. */
const UNDER_TEXT = IS_MOBILE ? 'w-full pl-9' : '';
const CARD_HAIRLINE = 'border-t border-textPrimary/[0.08]';
/** A field on the raised card: on a light theme the card's tint would show through the
 *  glass field and wash out its placeholder, so it sits on near-white there. */
const ON_RAISED_FIELD =
  '[html[data-theme-tone=light]_&]:!bg-white/80 [html[data-theme-tone=light]_&]:placeholder:!text-textSecondary';

/** Where rules can run, said once for this platform. */
function whereRulesRun(closeToTray: CloseToTrayMode): string {
  if (IS_MOBILE) return 'Rules run while StreamNook is open on this phone.';
  if (closeToTray === 'always') {
    return IS_MAC ? 'Rules keep running from the menu bar.' : 'Rules keep running from the tray.';
  }
  return 'Rules run while StreamNook is open.';
}

/** A rule open in the editor: a stored one, or a new one not saved yet. */
type Editing = { kind: 'rule'; id: string } | { kind: 'new'; rule: AutomationRule; n: number };

/** The limits every rule shares, folded behind one line saying what is set.
 *  The rows stay in the page while folded, so a settings search can find one:
 *  landing on a folded row opens the card. */
function LimitsCard({ summary, children }: { summary: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const reveal = () => setOpen(true);
    el.addEventListener(SETTINGS_REVEAL_EVENT, reveal);
    return () => el.removeEventListener(SETTINGS_REVEAL_EVENT, reveal);
  }, []);
  const body = (
    <>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="group -mx-4 flex w-[calc(100%+2rem)] items-center gap-3 px-4 py-3 text-left"
      >
        <span className={`${LEAD} flex shrink-0 justify-center text-accent-on-surface`}>
          <SlidersHorizontal size={IS_MOBILE ? 20 : 17} aria-hidden />
        </span>
        <span className="min-w-0 flex-1">
          <span className={`block ${IS_MOBILE ? 'text-[15px]' : 'text-[14px]'} font-medium text-textPrimary`}>
            Limits and this device
          </span>
          {!open && <span className="mt-0.5 block text-[12px] leading-relaxed text-textSecondary">{summary}</span>}
        </span>
        <ChevronDown size={16} className={`shrink-0 text-textMuted transition-transform ${open ? 'rotate-180' : ''}`} />
      </button>
      <div className={open ? (IS_MOBILE ? '' : 'border-t border-borderSubtle/55') : 'hidden'}>
        {children}
      </div>
    </>
  );
  return <div ref={ref}>{IS_MOBILE ? <GroupCard>{body}</GroupCard> : <div className="settings-card px-4">{body}</div>}</div>;
}

const AutomationsSettings = () => {
  const status = useAutomationStatusStore((s) => s.status);
  const prefs = useAppStore((s) => s.settings.automation_prefs);
  const closeToTray = useAppStore((s) => s.settings.close_to_tray ?? 'with-popouts');
  const initialRow = useAppStore((s) => s.settingsInitialRow);
  const openSettings = useAppStore((s) => s.openSettings);
  const resetFor = useSettingReset();
  const now = useNowMs();
  const [awayText, setAwayText] = useState('');
  const [problem, setProblem] = useState<string | null>(null);
  const [templatesOpen, setTemplatesOpen] = useState(false);
  // A settings link to one rule (`streamnook://settings/Automations?rule=<id>`)
  // arrives as the row `rule:<id>`, and opens that rule's editor.
  const linkedRule = initialRow?.startsWith('rule:') ? initialRow.slice('rule:'.length) : null;
  const [editing, setEditing] = useState<Editing | null>(linkedRule ? { kind: 'rule', id: linkedRule } : null);
  const [seenLink, setSeenLink] = useState(linkedRule);
  if (linkedRule !== seenLink) {
    setSeenLink(linkedRule);
    if (linkedRule) setEditing({ kind: 'rule', id: linkedRule });
  }
  const rootRef = useRef<HTMLDivElement>(null);
  const editingKey = editing ? (editing.kind === 'rule' ? editing.id : `new-${editing.n}`) : null;
  const wasEditing = useRef(false);

  useEffect(() => {
    void refreshAutomationStatus();
  }, []);

  // Opening a rule starts at the top of its editor; closing it lands back on the rules.
  useLayoutEffect(() => {
    if (editingKey) rootRef.current?.scrollIntoView({ block: 'start' });
    else if (wasEditing.current) document.getElementById(sectionIdFromLabel('Rules'))?.scrollIntoView({ block: 'start' });
    wasEditing.current = editingKey !== null;
  }, [editingKey]);

  const run = status?.run ?? 'off';
  const away = status?.away ?? null;
  const lastAway = status?.last_away_message ?? '';
  const spm = prefs?.sends_per_minute ?? 10;
  const other = prefs?.other_channels ?? false;
  const gap = prefs?.other_channel_gap_seconds ?? 30;
  const notes = prefs?.chat_notes ?? true;
  const whisperDay = prefs?.whisper_people_per_day ?? 20;
  const main = status?.accounts.find((a) => a.main) ?? null;

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
  const openNew = (rule: AutomationRule) => {
    setTemplatesOpen(false);
    setEditing((e) => ({ kind: 'new', rule, n: (e?.kind === 'new' ? e.n : 0) + 1 }));
  };

  const waiting = status?.outage
    ? "Can't reach StreamNook right now; rules wait."
    : status?.busy
      ? 'StreamNook is busy; rules wait a moment.'
      : null;
  const kept = status && run !== 'off' && status.chats_kept.length > 0 ? status.chats_kept.length : 0;

  // The rule editor takes the page's place while a rule is open.
  const openRow = editing?.kind === 'rule' ? status?.rules.find((r) => r.id === editing.id) : undefined;
  const initial = editing?.kind === 'new' ? editing.rule : openRow?.rule;
  if (status && editing && initial) {
    return (
      <div ref={rootRef} className="scroll-mt-4">
        <RuleEditor
          key={editingKey ?? ''}
          initial={initial}
          isNew={editing.kind === 'new'}
          summary={openRow?.summary ?? ''}
          accounts={status.accounts}
          platformAccounts={status.platform_accounts}
          otherChannelsOn={other}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            void refreshAutomationStatus();
          }}
          onDuplicate={(rule) => openNew(duplicateRule(rule))}
        />
      </div>
    );
  }

  const limitsSummary = [
    `${spm} messages a minute`,
    `${whisperDay} whispers a day`,
    other ? 'other channels too' : 'your channels only',
    notes ? 'chat notes on' : 'chat notes off',
    status?.device_name ? `"${status.device_name}"` : null,
  ]
    .filter(Boolean)
    .join(', ');

  return (
    <div ref={rootRef} className={IS_MOBILE ? 'space-y-7' : 'space-y-9'}>
      <SettingsSection label="Status" bare>
        <div>
          <div className={`${RAISED_CARD} ${!status ? 'pointer-events-none opacity-60' : ''}`}>
            <div data-setting-row="Status" className="flex flex-wrap items-center gap-x-3 gap-y-3 px-4 py-3.5">
              <span className={`${LEAD} flex shrink-0 justify-center`}>
                <RunDot run={run} />
              </span>
              <div className="min-w-0 flex-1">
                <div className={`${IS_MOBILE ? 'text-[16px]' : 'text-[15px]'} font-semibold text-textPrimary`}>{RUN_TITLES[run]}</div>
                <p className={`mt-0.5 ${HINT}`}>{RUN_WORDS[run]}</p>
                {waiting && <WarnLine className="mt-1.5">{waiting}</WarnLine>}
                {kept > 0 && (
                  <Note className="mt-1.5">
                    Rules keep {kept} {kept === 1 ? 'chat' : 'chats'} open to listen in.
                  </Note>
                )}
                {problem && <ErrorLine className="mt-1.5">{problem}</ErrorLine>}
              </div>
              <div className={UNDER_TEXT}>
                <SegmentedSelect
                  value={run}
                  options={RUN_OPTIONS}
                  fullWidth={IS_MOBILE}
                  onChange={(state) => act(automationsApi.setRun(state))}
                />
              </div>
            </div>
            <div data-setting-row="Away" className={`${CARD_HAIRLINE} flex flex-wrap items-center gap-x-3 gap-y-2.5 px-4 py-3`}>
              <span className={`${LEAD} flex shrink-0 justify-center text-accent-on-surface`}>
                <Coffee size={IS_MOBILE ? 20 : 17} aria-hidden />
              </span>
              <div className="min-w-0 flex-1">
                <div className={`${IS_MOBILE ? 'text-[15px]' : 'text-[14px]'} font-medium text-textPrimary`}>
                  {away ? `Away for ${spanText(away.since_ms, now)}` : 'Away'}
                </div>
                <p className={`mt-0.5 ${HINT}`}>
                  {away
                    ? `"${away.message}"`
                    : 'Rules marked "Only while you\'re away" answer for you. /afk and /back work too.'}
                </p>
              </div>
              <div className={`flex items-center gap-2 ${UNDER_TEXT}`}>
                {away ? (
                  <button type="button" onClick={() => act(automationsApi.setAway(null))} className={QUIET_BUTTON}>
                    I'm back
                  </button>
                ) : (
                  <>
                    <input
                      value={awayText}
                      maxLength={200}
                      aria-label="Away message"
                      onChange={(e) => setAwayText(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') setAway();
                      }}
                      placeholder={lastAway || DEFAULT_AWAY_MESSAGE}
                      className={`${IS_MOBILE ? `${INPUT} min-w-0 flex-1` : `${FIELD} w-52`} ${ON_RAISED_FIELD}`}
                    />
                    <button type="button" onClick={setAway} className={QUIET_BUTTON}>
                      Set away
                    </button>
                  </>
                )}
              </div>
            </div>
          </div>
          <Note className={IS_MOBILE ? 'px-4 pt-2.5' : 'px-1 pt-2.5'}>
            {whereRulesRun(closeToTray)} On more than one device, the first to see something answers.
          </Note>
        </div>
      </SettingsSection>

      <SettingsSection
        label="Reminders"
        id="automation-reminders"
        count={status ? (status.reminders ?? []).length : undefined}
        description="Lines posted on a timer, or when someone says a word. They follow their own switches, not the status above."
        actions={
          <button type="button" onClick={() => openSettings('Chat', 'reminders')} className={GHOST_BUTTON}>
            <Plus size={14} />
            New reminder
          </button>
        }
        bare
      >
        <RemindersGroup status={status} now={now} />
      </SettingsSection>

      <SettingsSection
        label="Rules"
        count={status?.rules.length}
        description="Each rule waits for something, checks its conditions, then acts. New rules start in Test."
        actions={
          <RuleHeaderActions
            canAdd={main !== null}
            templatesOpen={templatesOpen}
            onTemplates={() => setTemplatesOpen((v) => !v)}
            onNew={() => main && openNew(blankRule(accountRef(main)))}
          />
        }
        bare
      >
        {templatesOpen && (
          <TemplatePicker
            onAdded={(id) => {
              setTemplatesOpen(false);
              setEditing({ kind: 'rule', id });
            }}
          />
        )}
        <RuleList
          status={status}
          now={now}
          onOpen={(id) => setEditing({ kind: 'rule', id })}
          onDuplicate={(rule) => openNew(duplicateRule(rule))}
        />
      </SettingsSection>

      <SettingsSection label="Activity" shownLabel="What your rules did" bare>
        <ActivityCard now={now} status={status} />
      </SettingsSection>

      <SettingsSection label="Limits" bare>
        <LimitsCard summary={limitsSummary}>
          <SettingsRow
            title="Messages a minute"
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
            title="New people to whisper a day"
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
            title="Post in other channels"
            description="Rules may post in chats that aren't yours, at most once per chat in each gap."
            onReset={resetFor('automation_prefs.other_channels')}
            control={<Toggle enabled={other} onChange={() => write('other_channels', !other)} ariaLabel="Post in other channels" />}
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
            title="This device's name"
            description="Shown on your other devices when this one answers first."
            disabled={!status}
            control={
              <EntryInput
                value={status?.device_name ?? ''}
                save={saveDeviceName}
                maxLength={40}
                aria-label="This device's name"
                spellCheck={false}
                className={`${FIELD} ${IS_MOBILE ? 'w-40' : 'w-48'}`}
              />
            }
          />
        </LimitsCard>
      </SettingsSection>
    </div>
  );
};

export default AutomationsSettings;
