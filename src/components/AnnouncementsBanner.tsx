import { useEffect, useState, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { AlertTriangle, AlertOctagon, X, ExternalLink } from 'lucide-react';
import { Logger } from '../utils/logger';
import { afterBoot } from '../utils/startupScheduler';

interface AnnouncementAction {
  label: string;
  url: string;
}

interface Announcement {
  id: string;
  severity: 'critical' | 'warning' | 'info' | string;
  title: string;
  body: string;
  min_version?: string | null;
  max_version?: string | null;
  dismissible?: boolean | null;
  action?: AnnouncementAction | null;
}

interface AnnouncementsFile {
  version: number;
  announcements: Announcement[];
}

const POLL_INTERVAL_MS = 30 * 60 * 1000;
const DISMISSED_STORAGE_KEY = 'streamnook_dismissed_announcements_v1';

// Set by the setup wizard when a brand-new user finishes onboarding. On the next
// announcements fetch we fold whatever is currently live into the dismissed set,
// so a fresh install starts clean and only sees announcements published after
// signup. The existing backlog was meant for users who were already here.
export const ANNOUNCEMENTS_BASELINE_PENDING_KEY = 'streamnook_announcements_baseline_pending_v1';

const compareVersions = (a: string, b: string): number => {
  const pa = a.split('.').map((n) => parseInt(n, 10) || 0);
  const pb = b.split('.').map((n) => parseInt(n, 10) || 0);
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const x = pa[i] ?? 0;
    const y = pb[i] ?? 0;
    if (x !== y) return x - y;
  }
  return 0;
};

const loadDismissed = (): Set<string> => {
  try {
    const raw = localStorage.getItem(DISMISSED_STORAGE_KEY);
    if (!raw) return new Set();
    const arr = JSON.parse(raw);
    return new Set(Array.isArray(arr) ? arr : []);
  } catch {
    return new Set();
  }
};

const saveDismissed = (ids: Set<string>) => {
  try {
    localStorage.setItem(DISMISSED_STORAGE_KEY, JSON.stringify(Array.from(ids)));
  } catch {
    // localStorage full or unavailable; lose the dismissal silently
  }
};

// Solid, opaque tones — framing via an amber rim, a left inset bevel bar and a
// dark drop shadow (no outer glow). The fill is near-opaque so the box stays
// readable regardless of the glass slider, which strips backdrop blur at 0%.
const severityTone = (severity: string) => {
  switch (severity) {
    case 'critical':
      return {
        bg: 'rgba(40, 14, 12, 0.97)',
        border: 'border-red-400/55',
        bar: '#f87171',
        icon: 'text-red-300',
        title: 'text-red-50',
        body: 'text-red-100/85',
        link: 'text-red-200 hover:text-red-100',
        Icon: AlertOctagon,
      };
    // warning + info both render as the amber box.
    default:
      return {
        bg: 'rgba(46, 32, 8, 0.97)',
        border: 'border-amber-400/55',
        bar: '#fbbf24',
        icon: 'text-amber-300',
        title: 'text-amber-50',
        body: 'text-amber-100/85',
        link: 'text-amber-200 hover:text-amber-100',
        Icon: AlertTriangle,
      };
  }
};

const AnnouncementsBanner = () => {
  const [announcements, setAnnouncements] = useState<Announcement[]>([]);
  const [binaryVersion, setBinaryVersion] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState<Set<string>>(() => loadDismissed());

  const fetchOnce = useCallback(async () => {
    try {
      const file = await invoke<AnnouncementsFile>('fetch_announcements');
      const list = file.announcements ?? [];
      // Fresh install just finished onboarding: treat everything live right now
      // as already seen so the backlog doesn't dump on a first-time user.
      if (localStorage.getItem(ANNOUNCEMENTS_BASELINE_PENDING_KEY)) {
        setDismissed((prev) => {
          const next = new Set(prev);
          list.forEach((a) => next.add(a.id));
          saveDismissed(next);
          return next;
        });
        localStorage.removeItem(ANNOUNCEMENTS_BASELINE_PENDING_KEY);
      }
      setAnnouncements(list);
    } catch (e) {
      Logger.warn('fetch_announcements failed:', e);
    }
  }, []);

  useEffect(() => {
    (async () => {
      try {
        const v = await invoke<string>('get_app_version');
        setBinaryVersion(v);
      } catch (e) {
        Logger.warn('get_app_version failed (binary version unknown):', e);
      }
    })();
  }, []);

  useEffect(() => {
    // First fetch rides the boot stagger; the 30-min poll cadence is unchanged.
    const cancel = afterBoot(5000, () => {
      void fetchOnce();
    });
    const id = setInterval(fetchOnce, POLL_INTERVAL_MS);
    return () => {
      cancel();
      clearInterval(id);
    };
  }, [fetchOnce]);

  const dismiss = (id: string) => {
    setDismissed((prev) => {
      const next = new Set(prev);
      next.add(id);
      saveDismissed(next);
      return next;
    });
  };

  const handleAction = async (url: string) => {
    try {
      await invoke('open_browser_url', { url });
    } catch (e) {
      Logger.error('open_browser_url failed:', e);
    }
  };

  const visible = announcements.filter((a) => {
    if (dismissed.has(a.id)) return false;
    if (binaryVersion) {
      if (a.min_version && compareVersions(binaryVersion, a.min_version) < 0) return false;
      if (a.max_version && compareVersions(binaryVersion, a.max_version) > 0) return false;
    }
    return true;
  });

  if (visible.length === 0) return null;

  return (
    <div className="fixed top-12 right-4 z-[150] flex flex-col gap-2 max-w-sm pointer-events-none">
      <AnimatePresence>
        {visible.map((a) => {
          const tone = severityTone(a.severity);
          const { Icon } = tone;
          const allowDismiss = a.dismissible !== false;
          return (
            <motion.div
              key={a.id}
              initial={{ opacity: 0, y: -8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              style={{
                background: tone.bg,
                boxShadow: `inset 3px 0 0 0 ${tone.bar}, 0 8px 24px rgba(0, 0, 0, 0.55)`,
              }}
              className={`pointer-events-auto rounded-lg border ${tone.border} p-3.5 pl-4`}
            >
              <div className="flex items-start gap-3">
                <div className={`mt-0.5 ${tone.icon}`}>
                  <Icon size={16} />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center justify-between gap-2">
                    <h3 className={`text-sm font-semibold ${tone.title}`}>{a.title}</h3>
                    {allowDismiss && (
                      <button
                        onClick={() => dismiss(a.id)}
                        className="text-textSecondary hover:text-textPrimary"
                        aria-label="Dismiss"
                      >
                        <X size={14} />
                      </button>
                    )}
                  </div>
                  <p className={`mt-1 text-xs leading-relaxed whitespace-pre-line ${tone.body}`}>
                    {a.body}
                  </p>
                  {a.action && (
                    <button
                      onClick={() => handleAction(a.action!.url)}
                      className={`mt-2 inline-flex items-center gap-1 text-xs hover:underline ${tone.link}`}
                    >
                      {a.action.label} <ExternalLink size={11} />
                    </button>
                  )}
                </div>
              </div>
            </motion.div>
          );
        })}
      </AnimatePresence>
    </div>
  );
};

export default AnnouncementsBanner;
