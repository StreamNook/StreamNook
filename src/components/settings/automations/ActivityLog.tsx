// What your rules did, in one card with two tabs. Activity: what rules did,
// would do in Test, and skipped, newest first, grouped Today and Earlier, with
// a filter and Clear; the newest few show until "Show all activity". New
// entries arrive while the page is open: Rust sends them only to the windows
// watching, so this window listens to its own events. Try a message runs a
// sample against every rule (TryIt).
import { useEffect, useEffectEvent, useState } from 'react';
import { Check, ChevronRight, CircleSlash, FlaskConical, History, MonitorSmartphone, Trash2, type LucideIcon } from 'lucide-react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { Dropdown } from '../../ui/Dropdown';
import { IS_MOBILE } from '../../../utils/platform';
import {
  ACTIVITY_PAGE,
  AUTOMATION_ACTIVITY_EVENT,
  applyEntry,
  automationsApi,
  chatLabel,
  clockText,
  dayText,
  errorText,
  groupActivity,
  type AutomationActivityBatch,
  type AutomationActivityEntry,
  type AutomationActivityFilter,
  type AutomationActivityKind,
  type AutomationStatus,
} from '../../../utils/automations';
import TryIt from './TryIt';
import { CARD, DANGER_BUTTON, ErrorLine, GHOST_BUTTON, LIGHT_OK, LIGHT_WARN, META, QUIET_BUTTON } from './parts';

const FILTERS: { value: AutomationActivityFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'acted', label: 'Acted' },
  { value: 'test', label: 'Test' },
  { value: 'skipped', label: 'Skipped' },
];

const EMPTY: Record<AutomationActivityFilter, string> = {
  all: 'Nothing yet. When a rule acts, or would act in Test, it shows here.',
  acted: 'No rule has acted yet.',
  test: 'No Test results yet.',
  skipped: 'Nothing was skipped.',
};

/** The mark of each kind of entry: a check for done, a flask for Test, a slash for skipped, a device for another device's note. */
const KIND_MARK: Record<AutomationActivityKind, { icon: LucideIcon; tone: string }> = {
  acted: { icon: Check, tone: `text-success ${LIGHT_OK}` },
  test: { icon: FlaskConical, tone: `text-warning ${LIGHT_WARN}` },
  skipped: { icon: CircleSlash, tone: 'text-textMuted' },
  notice: { icon: MonitorSmartphone, tone: 'text-textMuted' },
};

/** How many entries show before "Show all activity". */
const FIRST_FEW = 5;

const LEAD = IS_MOBILE ? 'w-6' : 'w-[18px]';
/** "Show all activity": a link in the accent, not a quiet button. */
const MORE_LINK =
  'inline-flex items-center gap-1 rounded-lg px-2 py-1.5 text-[13px] font-medium text-accent-on-surface transition-colors hover:bg-textPrimary/[0.06]';

function EntryRow({ entry, earlier }: { entry: AutomationActivityEntry; earlier: boolean }) {
  const tag = entry.rule_id.startsWith('reminder:') ? 'Reminder' : null;
  const where = [tag, entry.rule_name, entry.channel ? chatLabel(entry.channel) : null].filter(Boolean).join(' · ');
  const mark = KIND_MARK[entry.kind];
  const Mark = mark.icon;
  return (
    <div className="flex items-start gap-3 px-4 py-2.5">
      <span className={`${LEAD} flex shrink-0 justify-center pt-[2px] ${mark.tone}`}>
        <Mark size={15} aria-hidden />
      </span>
      <div className="min-w-0 flex-1">
        <p className="text-[13px] leading-relaxed text-textPrimary">
          {entry.text}
          {entry.count > 1 && <span className="ml-1.5 text-textMuted">×{entry.count}</span>}
        </p>
        {where && <p className={`truncate ${META}`}>{where}</p>}
      </div>
      <time className="shrink-0 pt-0.5 text-[11.5px] tabular-nums text-textMuted">
        {earlier ? `${dayText(entry.at_ms)}, ${clockText(entry.at_ms)}` : clockText(entry.at_ms)}
      </time>
    </div>
  );
}

function Day({ title, entries, earlier }: { title: string; entries: AutomationActivityEntry[]; earlier: boolean }) {
  return (
    <div>
      <h4 className="px-4 pb-0.5 pt-3 text-[11.5px] font-semibold text-textMuted">{title}</h4>
      <div className="automation-hairlines">
        {entries.map((e) => (
          <EntryRow key={e.id} entry={e} earlier={earlier} />
        ))}
      </div>
    </div>
  );
}

function ActivityTab({ now, filter }: { now: number; filter: AutomationActivityFilter }) {
  const [entries, setEntries] = useState<AutomationActivityEntry[]>([]);
  const [more, setMore] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [all, setAll] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    automationsApi
      .activity(filter, ACTIVITY_PAGE, null)
      .then((list) => {
        if (!live) return;
        setEntries(list);
        setMore(list.length === ACTIVITY_PAGE);
        setProblem(null);
        setLoaded(true);
      })
      .catch((e: unknown) => {
        if (!live) return;
        setProblem(errorText(e));
        setLoaded(true);
      });
    return () => {
      live = false;
    };
  }, [filter]);

  const onEntry = useEffectEvent((entry: AutomationActivityEntry) => {
    setEntries((list) => applyEntry(list, entry, filter));
  });

  useEffect(() => {
    let stopped = false;
    let unlisten: (() => void) | undefined;
    void automationsApi.watchActivity(true).catch(() => {});
    // Window-scoped: a plain `listen` hears what Rust sends to every other window too.
    getCurrentWebviewWindow()
      .listen<AutomationActivityBatch>(AUTOMATION_ACTIVITY_EVENT, (e) => {
        // Oldest first, so the newest ends on top.
        for (const entry of e.payload.entries) onEntry(entry);
      })
      .then((fn) => {
        if (stopped) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      stopped = true;
      unlisten?.();
      void automationsApi.watchActivity(false).catch(() => {});
    };
  }, []);

  const loadMore = () => {
    const last = entries[entries.length - 1];
    if (!last) return;
    automationsApi
      .activity(filter, ACTIVITY_PAGE, last.id)
      .then((list) => {
        setEntries((shown) => [...shown, ...list.filter((e) => !shown.some((s) => s.id === e.id))]);
        setMore(list.length === ACTIVITY_PAGE);
      })
      .catch((e: unknown) => setProblem(errorText(e)));
  };

  const clear = () => {
    automationsApi
      .clearActivity()
      .then(() => {
        setEntries([]);
        setMore(false);
        setConfirmClear(false);
      })
      .catch((e: unknown) => setProblem(errorText(e)));
  };

  const shown = all ? entries : entries.slice(0, FIRST_FEW);
  const { today, earlier } = groupActivity(shown, now);
  const hidden = entries.length - shown.length;

  return (
    <div>
      {problem && <ErrorLine className="px-4 pt-3">{problem}</ErrorLine>}
      {loaded && entries.length === 0 && !problem && (
        <p className="px-4 py-6 text-center text-[13px] text-textSecondary">{EMPTY[filter]}</p>
      )}
      {today.length > 0 && <Day title="Today" entries={today} earlier={false} />}
      {earlier.length > 0 && <Day title="Earlier" entries={earlier} earlier />}
      {entries.length > 0 && (
        <div className="mt-1 flex flex-wrap items-center justify-between gap-2 border-t border-[color:var(--settings-hairline,color-mix(in_srgb,var(--color-border-subtle)_55%,transparent))] px-2.5 py-1.5">
          {hidden > 0 ? (
            <button type="button" onClick={() => setAll(true)} className={MORE_LINK}>
              Show all activity
              <ChevronRight size={14} />
            </button>
          ) : more ? (
            <button type="button" onClick={loadMore} className={MORE_LINK}>
              Show older
              <ChevronRight size={14} />
            </button>
          ) : (
            <span />
          )}
          {confirmClear ? (
            <span className="flex items-center gap-1.5">
              <button type="button" onClick={clear} className={DANGER_BUTTON}>
                Clear all activity
              </button>
              <button type="button" onClick={() => setConfirmClear(false)} className={QUIET_BUTTON}>
                Keep it
              </button>
            </span>
          ) : (
            <button type="button" onClick={() => setConfirmClear(true)} className={GHOST_BUTTON}>
              <Trash2 size={13} />
              Clear
            </button>
          )}
        </div>
      )}
    </div>
  );
}

type Tab = 'activity' | 'try';

const TABS: { value: Tab; label: string; icon: LucideIcon }[] = [
  { value: 'activity', label: 'Activity', icon: History },
  { value: 'try', label: 'Try a message', icon: FlaskConical },
];

export default function ActivityCard({ now, status }: { now: number; status: AutomationStatus | null }) {
  const [tab, setTab] = useState<Tab>('activity');
  const [filter, setFilter] = useState<AutomationActivityFilter>('all');
  return (
    <div className={CARD}>
      <div
        role="tablist"
        className="flex items-center gap-1 border-b border-[color:var(--settings-hairline,color-mix(in_srgb,var(--color-border-subtle)_55%,transparent))] p-1.5"
      >
        {TABS.map((t) => {
          const Icon = t.icon;
          const on = tab === t.value;
          return (
            <button
              key={t.value}
              type="button"
              role="tab"
              aria-selected={on}
              onClick={() => setTab(t.value)}
              className={`inline-flex items-center gap-1.5 rounded-lg px-3 text-[13px] font-medium transition-colors ${
                IS_MOBILE ? 'min-h-[40px]' : 'py-1.5'
              } ${on ? 'bg-textPrimary/[0.09] text-textPrimary' : 'text-textSecondary hover:text-textPrimary'}`}
            >
              <Icon size={14} aria-hidden />
              {t.label}
            </button>
          );
        })}
        {tab === 'activity' && (
          <div className="ml-auto">
            <Dropdown
              value={filter}
              options={FILTERS}
              onChange={setFilter}
              align="right"
              ariaLabel="Show"
              className="!py-1 text-[12.5px]"
            />
          </div>
        )}
      </div>
      {tab === 'activity' ? <ActivityTab now={now} filter={filter} /> : <TryIt status={status} />}
    </div>
  );
}
