// Activity: what rules did, would do in Test, and skipped, newest first,
// grouped Today and Earlier, with a filter and Clear. New entries arrive while
// the page is open: Rust sends them only to the windows watching, so this
// window listens to its own events.
import { useEffect, useEffectEvent, useState } from 'react';
import { Trash2 } from 'lucide-react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { SegmentedSelect } from '../_primitives';
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
} from '../../../utils/automations';
import { DANGER_BUTTON, QUIET_BUTTON } from './parts';

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

const KIND_DOT: Record<AutomationActivityKind, string> = {
  acted: 'bg-accent',
  test: 'border border-accent',
  skipped: 'bg-textMuted/60',
  notice: 'bg-textSecondary/50',
};

function EntryRow({ entry, earlier }: { entry: AutomationActivityEntry; earlier: boolean }) {
  const tag = entry.rule_id.startsWith('reminder:') ? 'Reminder' : null;
  const where = [tag, entry.rule_name, entry.channel ? chatLabel(entry.channel) : null].filter(Boolean).join(' · ');
  return (
    <div className="flex items-start gap-3 px-3.5 py-2.5">
      <span aria-hidden className={`mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full ${KIND_DOT[entry.kind]}`} />
      <div className="min-w-0 flex-1">
        <p className="text-[13px] leading-relaxed text-textPrimary">
          {entry.text}
          {entry.count > 1 && <span className="ml-1.5 text-textMuted">×{entry.count}</span>}
        </p>
        {where && <p className="mt-0.5 truncate text-[11.5px] text-textMuted">{where}</p>}
      </div>
      <time className="shrink-0 pt-0.5 text-[11.5px] tabular-nums text-textMuted">
        {earlier ? `${dayText(entry.at_ms)}, ${clockText(entry.at_ms)}` : clockText(entry.at_ms)}
      </time>
    </div>
  );
}

function Day({ title, entries, earlier }: { title: string; entries: AutomationActivityEntry[]; earlier: boolean }) {
  return (
    <div className="space-y-1.5">
      <h4 className="px-1 text-[11px] font-semibold uppercase tracking-[0.12em] text-textSecondary">{title}</h4>
      <div className="divide-y divide-borderSubtle/50 rounded-lg bg-glass/30">
        {entries.map((e) => (
          <EntryRow key={e.id} entry={e} earlier={earlier} />
        ))}
      </div>
    </div>
  );
}

export default function ActivityLog({ now }: { now: number }) {
  const [filter, setFilter] = useState<AutomationActivityFilter>('all');
  const [entries, setEntries] = useState<AutomationActivityEntry[]>([]);
  const [more, setMore] = useState(false);
  const [loaded, setLoaded] = useState(false);
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

  const { today, earlier } = groupActivity(entries, now);

  return (
    <div className="space-y-3">
      <div className={`flex flex-wrap items-center justify-between gap-3 ${IS_MOBILE ? 'px-1' : ''}`}>
        <SegmentedSelect value={filter} options={FILTERS} onChange={setFilter} fullWidth={IS_MOBILE} />
        {entries.length > 0 &&
          (confirmClear ? (
            <span className="flex items-center gap-1.5">
              <button type="button" onClick={clear} className={DANGER_BUTTON}>
                Clear all activity
              </button>
              <button type="button" onClick={() => setConfirmClear(false)} className={QUIET_BUTTON}>
                Keep it
              </button>
            </span>
          ) : (
            <button type="button" onClick={() => setConfirmClear(true)} className={QUIET_BUTTON}>
              <Trash2 size={13} />
              Clear
            </button>
          ))}
      </div>
      {problem && (
        <p role="alert" className="px-1 text-[12px] text-error">
          {problem}
        </p>
      )}
      {loaded && entries.length === 0 && !problem && (
        <div className="rounded-lg bg-glass/30 px-4 py-6 text-center text-sm text-textSecondary">{EMPTY[filter]}</div>
      )}
      {today.length > 0 && <Day title="Today" entries={today} earlier={false} />}
      {earlier.length > 0 && <Day title="Earlier" entries={earlier} earlier />}
      {more && (
        <button type="button" onClick={loadMore} className={QUIET_BUTTON}>
          Show older
        </button>
      )}
    </div>
  );
}
