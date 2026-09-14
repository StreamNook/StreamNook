import { useEffect, useMemo, useState } from 'react';
import {
  ChevronDown,
  ChevronUp,
  Plus,
  Trash2,
  Minus,
  BookOpen,
  Search,
  X,
} from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { SettingsSection, SegmentedSelect } from './_primitives';
import {
  useChannelSearch,
  type ChannelItem,
} from '../multi-nook/channelSearch';
import { ChannelResultRow } from '../multi-nook/ChannelResultRow';
import type {
  Reminder,
  ReminderTriggerType,
  ReminderKeywordMatch,
  ReminderKeywordFrom,
  ReminderChannelScope,
} from '../../types';
import type { ProviderId } from '../../types/providers';

const MAX_REPEAT = 10;

function makeDefaultReminder(): Reminder {
  return {
    id:
      typeof crypto !== 'undefined' && 'randomUUID' in crypto
        ? crypto.randomUUID()
        : `rmd-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    enabled: true,
    label: '',
    message: '',
    trigger: 'interval',
    repeat_count: 1,
    channel_scope: 'current',
    interval_seconds: 20 * 60,
    delay_seconds: 30 * 60,
    clock_time: '20:00',
    uptime_seconds: 60 * 60,
    keyword: '',
    keyword_match: 'contains',
    keyword_from: 'anyone',
    keyword_cooldown_minutes: 5,
  };
}

const TRIGGER_OPTIONS: { value: ReminderTriggerType; label: string }[] = [
  { value: 'interval', label: 'Interval' },
  { value: 'delay', label: 'Delay' },
  { value: 'clock', label: 'Time' },
  { value: 'uptime', label: 'Uptime' },
  { value: 'keyword', label: 'Keyword' },
];

const Tag: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <code className="px-1 py-0.5 bg-background/40 rounded text-textPrimary font-mono text-[11px]">{children}</code>
);

// Compact label + numeric stepper used by the minute-based triggers.
const MinutesField: React.FC<{
  label: string;
  value: number | undefined;
  onChange: (v: number) => void;
  suffix?: string;
}> = ({ label, value, onChange, suffix = 'minutes' }) => (
  <label className="flex items-center gap-2 text-[13px] text-textSecondary">
    <span>{label}</span>
    <input
      type="number"
      min={1}
      value={value ?? ''}
      onChange={(e) => onChange(Math.max(1, parseInt(e.target.value, 10) || 1))}
      className="w-16 glass-input text-textPrimary text-sm px-2 py-1 text-center"
    />
    <span>{suffix}</span>
  </label>
);

// Number + unit (seconds / minutes / hours) field. Stores the value in seconds
// so any amount can be set. Picks the friendliest unit on mount, then lets the
// user change either piece.
const UNIT_FACTOR: Record<'s' | 'm' | 'h', number> = { s: 1, m: 60, h: 3600 };
const DurationField: React.FC<{
  label: string;
  seconds: number;
  onChange: (seconds: number) => void;
  suffix?: string;
}> = ({ label, seconds, onChange, suffix }) => {
  const [state, setState] = useState<{ amount: number; unit: 's' | 'm' | 'h' }>(() => {
    const s = Math.max(0, Math.round(seconds || 0));
    if (s && s % 3600 === 0) return { amount: s / 3600, unit: 'h' };
    if (s && s % 60 === 0) return { amount: s / 60, unit: 'm' };
    return { amount: s || 1, unit: 's' };
  });
  const apply = (amount: number, unit: 's' | 'm' | 'h') => {
    setState({ amount, unit });
    onChange(Math.max(1, Math.round(amount * UNIT_FACTOR[unit])));
  };
  return (
    <label className="flex flex-wrap items-center gap-2 text-[13px] text-textSecondary">
      <span>{label}</span>
      <input
        type="number"
        min={1}
        value={state.amount}
        onChange={(e) => apply(Math.max(1, parseInt(e.target.value, 10) || 1), state.unit)}
        className="w-16 glass-input text-textPrimary text-sm px-2 py-1 text-center"
      />
      <select
        value={state.unit}
        onChange={(e) => apply(state.amount, e.target.value as 's' | 'm' | 'h')}
        className="glass-input text-textPrimary text-sm px-2 py-1"
      >
        <option value="s">seconds</option>
        <option value="m">minutes</option>
        <option value="h">hours</option>
      </select>
      {suffix && <span>{suffix}</span>}
    </label>
  );
};

const RepeatStepper: React.FC<{ value: number; onChange: (v: number) => void }> = ({ value, onChange }) => {
  const v = Math.min(MAX_REPEAT, Math.max(1, value || 1));
  return (
    <div className="flex items-center gap-1.5">
      <button
        type="button"
        onClick={() => onChange(Math.max(1, v - 1))}
        disabled={v <= 1}
        className="p-1 rounded text-textSecondary hover:text-textPrimary hover:bg-white/[0.06] disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
        aria-label="Fewer times"
      >
        <Minus size={13} />
      </button>
      <span className="w-5 text-center text-[13px] font-medium text-textPrimary tabular-nums">{v}</span>
      <button
        type="button"
        onClick={() => onChange(Math.min(MAX_REPEAT, v + 1))}
        disabled={v >= MAX_REPEAT}
        className="p-1 rounded text-textSecondary hover:text-textPrimary hover:bg-white/[0.06] disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
        aria-label="More times"
      >
        <Plus size={13} />
      </button>
    </div>
  );
};

/** Pinned at module scope so the array identity is stable across renders. */
const TWITCH_ONLY: ProviderId[] = ['twitch'];

// Inline channel search, reusing the same finder + row that back the MultiChat
// pickers (live follows first, then Twitch search, with avatars and live dots).
const ChannelPicker: React.FC<{
  login?: string;
  onPick: (login: string, id: string) => void;
}> = ({ login, onPick }) => {
  const [open, setOpen] = useState(false);
  const exclude = useMemo(() => new Set<string>(), []);
  // Twitch ONLY, deliberately. A reminder is stored as a bare channel_login and
  // fired through Twitch IRC, so a Kick row picked here would set a reminder that
  // silently never fires, or fires at a same-named Twitch channel instead.
  const {
    searchInput,
    setSearchInput,
    visibleItems,
    highlightIndex,
    setHighlightIndex,
    listRef,
    refreshFollowing,
    reset,
  } = useChannelSearch({ excludeKeys: exclude, providers: TWITCH_ONLY });

  useEffect(() => {
    if (open) void refreshFollowing();
  }, [open, refreshFollowing]);

  const select = (item: ChannelItem) => {
    // Search hits without a resolved id fall back to login==id; don't persist
    // that as a real channel id.
    onPick(item.login, item.source === 'search' && item.id === item.login ? '' : item.id);
    reset();
    setOpen(false);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setHighlightIndex((i) => Math.min(i + 1, Math.max(visibleItems.length - 1, 0)));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setHighlightIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const target = visibleItems[highlightIndex];
      if (target) select(target);
    } else if (e.key === 'Escape') {
      setOpen(false);
    }
  };

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="inline-flex items-center gap-2 glass-input text-textPrimary text-sm px-2.5 py-1.5 hover:bg-white/[0.05] transition-colors"
      >
        <Search size={13} className="text-textMuted" />
        {login ? `#${login}` : 'Choose a channel'}
      </button>
    );
  }

  return (
    <div className="rounded-lg border border-borderSubtle bg-background/40 p-2">
      <div className="relative mb-1.5">
        <Search size={13} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-textMuted" />
        <input
          autoFocus
          type="text"
          value={searchInput}
          onChange={(e) => setSearchInput(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="Search channels"
          className="w-full glass-input text-textPrimary text-sm pl-7 pr-7 py-1.5"
          spellCheck={false}
        />
        <button
          type="button"
          onClick={() => {
            reset();
            setOpen(false);
          }}
          className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-textMuted hover:bg-white/[0.06] hover:text-textPrimary"
          aria-label="Close"
        >
          <X size={12} />
        </button>
      </div>
      <div ref={listRef} className="max-h-52 overflow-y-auto scrollbar-thin space-y-0.5">
        {visibleItems.map((item, i) => (
          <ChannelResultRow
            key={`${item.login}-${i}`}
            item={item}
            index={i}
            highlighted={highlightIndex === i}
            onSelect={select}
            onHover={setHighlightIndex}
          />
        ))}
        {visibleItems.length === 0 && (
          <div className="px-3 py-3 text-center text-[11px] text-textMuted">
            {searchInput.trim() ? 'No channels found.' : 'Type to search, or pick a live follow.'}
          </div>
        )}
      </div>
    </div>
  );
};

const RemindersSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const currentStream = useAppStore((s) => s.currentStream);
  const [showDocs, setShowDocs] = useState(false);

  const reminders = useMemo(
    () => settings.reminders?.reminders ?? [],
    [settings.reminders],
  );

  const writeReminders = (next: Reminder[]) => {
    updateSettings({ ...settings, reminders: { reminders: next } });
  };

  const updateReminder = (id: string, patch: Partial<Reminder>) => {
    writeReminders(reminders.map((r) => (r.id === id ? { ...r, ...patch } : r)));
  };

  const removeReminder = (id: string) => {
    writeReminders(reminders.filter((r) => r.id !== id));
  };

  const moveReminder = (id: string, direction: -1 | 1) => {
    const idx = reminders.findIndex((r) => r.id === id);
    if (idx === -1) return;
    const target = idx + direction;
    if (target < 0 || target >= reminders.length) return;
    const next = reminders.slice();
    [next[idx], next[target]] = [next[target], next[idx]];
    writeReminders(next);
  };

  const addReminder = () => {
    writeReminders([...reminders, makeDefaultReminder()]);
  };

  const setScope = (reminder: Reminder, scope: ReminderChannelScope) => {
    if (scope === 'specific' && !reminder.channel_login && currentStream?.user_login) {
      // Pre-fill with the channel you're watching so "specific" is useful instantly.
      updateReminder(reminder.id, {
        channel_scope: scope,
        channel_login: currentStream.user_login.toLowerCase(),
        channel_id: currentStream.user_id,
      });
    } else {
      updateReminder(reminder.id, { channel_scope: scope });
    }
  };

  return (
    <SettingsSection label="Reminders" id="reminders" bare>
      <div className="flex items-start justify-between gap-3">
        <p className="text-xs text-textSecondary flex-1">
          Post a message into a streamer&apos;s chat on your behalf to nudge them about something: on a
          repeating timer, after a delay, at a set time, once the stream hits an uptime, or when a keyword shows up
          in chat. You can also fire one off instantly with <Tag>/remind now &lt;message&gt;</Tag>.
        </p>
        <button
          onClick={() => setShowDocs((v) => !v)}
          className="inline-flex items-center gap-1.5 px-2 py-1 text-xs text-textSecondary hover:text-textPrimary transition-colors flex-shrink-0"
        >
          <BookOpen size={12} />
          <span>{showDocs ? 'Hide reference' : 'Show reference'}</span>
        </button>
      </div>

      {showDocs && (
        <div className="mb-4 bg-glass/30 rounded-lg p-5 text-xs leading-relaxed space-y-6">
          <section>
            <h4 className="text-textPrimary font-medium mb-2">The five triggers</h4>
            <dl className="space-y-1.5">
              <div className="flex gap-3">
                <dt className="min-w-[5.5rem] text-textPrimary">Interval</dt>
                <dd className="text-textSecondary">posts on a repeating timer (any amount, down to seconds) the whole time you&apos;re in that chat.</dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[5.5rem] text-textPrimary">Delay</dt>
                <dd className="text-textSecondary">posts once, a set time (seconds, minutes, or hours) after you join the chat.</dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[5.5rem] text-textPrimary">Time</dt>
                <dd className="text-textSecondary">posts once a day at a set clock time (24-hour, your local time).</dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[5.5rem] text-textPrimary">Uptime</dt>
                <dd className="text-textSecondary">posts once when the stream you&apos;re watching has been live a set length.</dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[5.5rem] text-textPrimary">Keyword</dt>
                <dd className="text-textSecondary">posts when a chat message matches a word or phrase you set.</dd>
              </div>
            </dl>
          </section>

          <section className="pt-5 border-t border-borderSubtle/40">
            <h4 className="text-textPrimary font-medium mb-2">Posting it more than once</h4>
            <p className="text-textSecondary">
              Bump <span className="text-textPrimary">Repeat</span> to post the same line several times in a row so the
              streamer actually catches it. The copies carry an invisible character so Twitch doesn&apos;t reject them as
              duplicates, and they look identical in chat.
            </p>
          </section>

          <section className="pt-5 border-t border-borderSubtle/40">
            <h4 className="text-textPrimary font-medium mb-2">Auto-filled bits in the message</h4>
            <p className="text-textSecondary mb-2">Same placeholders as custom commands. Handy for @mentioning the streamer:</p>
            <dl className="space-y-1.5">
              <div className="flex gap-3">
                <dt className="min-w-[7.5rem]"><Tag>{'{channel}'}</Tag></dt>
                <dd className="text-textSecondary">the streamer&apos;s name</dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[7.5rem]"><Tag>{'{stream.uptime}'}</Tag></dt>
                <dd className="text-textSecondary">how long they&apos;ve been live, like <span className="text-textPrimary">2h 14m</span></dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[7.5rem]"><Tag>{'{stream.title}'}</Tag></dt>
                <dd className="text-textSecondary">the current title</dd>
              </div>
            </dl>
            <p className="text-textMuted mt-3">Stream placeholders come back empty for a pinned channel you aren&apos;t watching.</p>
          </section>

          <section className="pt-5 border-t border-borderSubtle/40">
            <h4 className="text-textPrimary font-medium mb-2">Setting one from chat</h4>
            <p className="text-textSecondary">
              Type <Tag>/remind every 20m Plug the Discord</Tag> to make one without opening settings. Also works with
              <Tag>in 30m</Tag>, <Tag>at 21:00</Tag>, <Tag>uptime 2h</Tag>, <Tag>on &lt;word&gt;</Tag>, and
              <Tag>now</Tag>. Add <Tag>x3</Tag> anywhere to post it three times.
            </p>
          </section>
        </div>
      )}

      <div className="space-y-3">
        {reminders.length === 0 && (
          <div className="bg-glass/30 rounded-lg px-4 py-6 text-center">
            <p className="text-sm text-textSecondary mb-3">No reminders yet.</p>
            <button
              onClick={addReminder}
              className="glass-button inline-flex items-center gap-2 px-3 py-1.5 rounded-lg text-textPrimary text-sm font-medium"
            >
              <Plus size={14} />
              Add your first reminder
            </button>
          </div>
        )}

        {reminders.map((reminder, idx) => (
          <div key={reminder.id} className="bg-glass/30 rounded-lg p-3 space-y-3">
            {/* Header: enable · name · reorder · delete */}
            <div className="flex items-center gap-2">
              <button
                onClick={() => updateReminder(reminder.id, { enabled: !reminder.enabled })}
                className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors flex-shrink-0 ${
                  reminder.enabled ? 'bg-accent' : 'bg-gray-600'
                }`}
                aria-label={reminder.enabled ? 'Disable reminder' : 'Enable reminder'}
              >
                <span
                  className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                    reminder.enabled ? 'translate-x-5' : 'translate-x-1'
                  }`}
                />
              </button>

              <input
                type="text"
                value={reminder.label ?? ''}
                onChange={(e) => updateReminder(reminder.id, { label: e.target.value })}
                placeholder="Reminder name (optional)"
                aria-label="Reminder name"
                maxLength={60}
                className="flex-1 bg-background/40 rounded border border-borderSubtle text-textPrimary text-sm px-2.5 py-1.5 focus:outline-none focus:border-white/[0.16]"
                spellCheck={false}
              />

              <div className="flex items-center gap-1 flex-shrink-0">
                <button
                  onClick={() => moveReminder(reminder.id, -1)}
                  disabled={idx === 0}
                  className="p-1 text-textSecondary hover:text-textPrimary disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                  aria-label="Move up"
                >
                  <ChevronUp size={14} />
                </button>
                <button
                  onClick={() => moveReminder(reminder.id, 1)}
                  disabled={idx === reminders.length - 1}
                  className="p-1 text-textSecondary hover:text-textPrimary disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                  aria-label="Move down"
                >
                  <ChevronDown size={14} />
                </button>
                <button
                  onClick={() => removeReminder(reminder.id)}
                  className="p-1 text-textSecondary hover:text-red-400 transition-colors"
                  aria-label="Delete reminder"
                >
                  <Trash2 size={14} />
                </button>
              </div>
            </div>

            {/* The message */}
            <label className="block">
              <span className="mb-1 block text-[11px] text-textMuted">Message to post</span>
              <textarea
                value={reminder.message}
                onChange={(e) => updateReminder(reminder.id, { message: e.target.value })}
                placeholder="What to post in chat. Supports {channel}, {stream.uptime}, {stream.title}."
                rows={2}
                className="w-full glass-input text-textPrimary text-sm px-2.5 py-1.5 resize-y"
                spellCheck={false}
              />
            </label>

            {/* Trigger picker */}
            <SegmentedSelect
              value={reminder.trigger}
              options={TRIGGER_OPTIONS}
              onChange={(v) => updateReminder(reminder.id, { trigger: v })}
            />

            {/* Trigger-specific fields */}
            <div className="px-0.5">
              {reminder.trigger === 'interval' && (
                <DurationField
                  label="Post every"
                  seconds={reminder.interval_seconds ?? (reminder.interval_minutes ?? 0) * 60}
                  onChange={(s) => updateReminder(reminder.id, { interval_seconds: s, interval_minutes: undefined })}
                />
              )}
              {reminder.trigger === 'delay' && (
                <DurationField
                  label="Post once,"
                  suffix="after I join"
                  seconds={reminder.delay_seconds ?? (reminder.delay_minutes ?? 0) * 60}
                  onChange={(s) => updateReminder(reminder.id, { delay_seconds: s, delay_minutes: undefined })}
                />
              )}
              {reminder.trigger === 'clock' && (
                <label className="flex items-center gap-2 text-[13px] text-textSecondary">
                  <span>Post at</span>
                  <input
                    type="text"
                    inputMode="numeric"
                    value={reminder.clock_time ?? ''}
                    onChange={(e) => updateReminder(reminder.id, { clock_time: e.target.value })}
                    placeholder="20:00"
                    maxLength={5}
                    className="w-20 glass-input text-textPrimary text-sm px-2 py-1 text-center"
                  />
                  <span>each day (24-hour, local time)</span>
                </label>
              )}
              {reminder.trigger === 'uptime' && (
                <DurationField
                  label="Post when live for"
                  seconds={reminder.uptime_seconds ?? (reminder.uptime_minutes ?? 0) * 60}
                  onChange={(s) => updateReminder(reminder.id, { uptime_seconds: s, uptime_minutes: undefined })}
                />
              )}
              {reminder.trigger === 'keyword' && (
                <div className="space-y-2">
                  <label className="block">
                    <span className="mb-1 block text-[11px] text-textMuted">Keyword</span>
                    <input
                      type="text"
                      value={reminder.keyword ?? ''}
                      onChange={(e) => updateReminder(reminder.id, { keyword: e.target.value })}
                      placeholder="Word or phrase to watch for"
                      className="w-full glass-input text-textPrimary text-sm px-2.5 py-1.5"
                      spellCheck={false}
                    />
                  </label>
                  <div className="flex flex-wrap items-center gap-2 text-[13px] text-textSecondary">
                    <span>Match</span>
                    <select
                      value={reminder.keyword_match ?? 'contains'}
                      onChange={(e) => updateReminder(reminder.id, { keyword_match: e.target.value as ReminderKeywordMatch })}
                      className="glass-input text-textPrimary text-sm px-2 py-1"
                    >
                      <option value="contains">contains it</option>
                      <option value="word">as a whole word</option>
                      <option value="exact">the exact message</option>
                    </select>
                    <span>from</span>
                    <select
                      value={reminder.keyword_from ?? 'anyone'}
                      onChange={(e) => updateReminder(reminder.id, { keyword_from: e.target.value as ReminderKeywordFrom })}
                      className="glass-input text-textPrimary text-sm px-2 py-1"
                    >
                      <option value="anyone">anyone</option>
                      <option value="broadcaster">the streamer</option>
                      <option value="mods">mods or streamer</option>
                    </select>
                  </div>
                  <MinutesField
                    label="At most once per"
                    suffix="minutes"
                    value={reminder.keyword_cooldown_minutes ?? 5}
                    onChange={(v) => updateReminder(reminder.id, { keyword_cooldown_minutes: v })}
                  />
                </div>
              )}
            </div>

            {/* Repeat + channel scope */}
            <div className="flex flex-wrap items-center justify-between gap-3 pt-1 border-t border-borderSubtle/40">
              <div className="flex items-center gap-2 text-[13px] text-textSecondary">
                <span>Repeat</span>
                <RepeatStepper
                  value={reminder.repeat_count ?? 1}
                  onChange={(v) => updateReminder(reminder.id, { repeat_count: v })}
                />
                <span>{(reminder.repeat_count ?? 1) > 1 ? 'times' : 'time'}</span>
              </div>
              <div className="flex items-center gap-2">
                <SegmentedSelect
                  value={reminder.channel_scope ?? 'current'}
                  options={[
                    { value: 'current', label: 'Current channel' },
                    { value: 'specific', label: 'Specific channel' },
                  ]}
                  onChange={(v) => setScope(reminder, v)}
                />
              </div>
            </div>

            {reminder.channel_scope === 'specific' && (
              <ChannelPicker
                login={reminder.channel_login}
                onPick={(login, id) =>
                  updateReminder(reminder.id, { channel_login: login, channel_id: id || undefined })
                }
              />
            )}
          </div>
        ))}

        {reminders.length > 0 && (
          <button
            onClick={addReminder}
            className="inline-flex items-center gap-2 px-3 py-1.5 text-textSecondary hover:text-textPrimary text-sm transition-colors"
          >
            <Plus size={14} />
            Add another reminder
          </button>
        )}
      </div>
    </SettingsSection>
  );
};

export default RemindersSettings;
