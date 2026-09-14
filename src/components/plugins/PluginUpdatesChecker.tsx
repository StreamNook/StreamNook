// Headless, app-level. Periodically compares installed plugins against every
// source's catalog and records which have a newer version available, so the
// title bar's Marketplace button can show an "updates available" badge. Reuses
// the same commands and version compare the marketplace browse uses, so the
// badge and the in-marketplace "Update" buttons always agree.

import { useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { compareVersions, type PluginInfo, type SourceInfo, type IndexEntry } from '../../types/plugins';
import { usePluginUpdates } from '../../stores/pluginUpdatesStore';
import { afterBoot } from '../../utils/startupScheduler';

const CHECK_INTERVAL_MS = 30 * 60 * 1000; // re-check every 30 minutes
const MIN_GAP_MS = 2 * 60 * 1000; // throttle focus/interval re-checks

export default function PluginUpdatesChecker() {
  useEffect(() => {
    let alive = true;
    let lastRun = 0;
    let running = false;

    // force bypasses the throttle (install/enable/disable events and the first
    // run should reflect immediately, e.g. to clear the badge right after an update).
    const check = async (force = false) => {
      if (!alive || running) return;
      if (!force && Date.now() - lastRun < MIN_GAP_MS) return;
      running = true;
      lastRun = Date.now();
      try {
        const [installed, sources] = await Promise.all([
          invoke<PluginInfo[]>('plugins_list'),
          invoke<SourceInfo[]>('plugins_sources'),
        ]);
        // Official source first, so it wins when the same id appears in several.
        const ordered = [...sources].sort((a, b) => Number(b.official) - Number(a.official));
        const latest = new Map<string, string>();
        for (const s of ordered) {
          try {
            const entries = await invoke<IndexEntry[]>('plugins_browse_source', { url: s.url });
            for (const e of entries) if (!latest.has(e.id)) latest.set(e.id, e.version);
          } catch {
            /* an unreachable source contributes nothing */
          }
        }
        const ids = installed
          .filter((p) => {
            const v = latest.get(p.id);
            return !!v && compareVersions(v, p.version) > 0;
          })
          .map((p) => p.id);
        if (alive) usePluginUpdates.getState().setIds(ids);
      } catch {
        /* plugin host unavailable */
      } finally {
        running = false;
      }
    };

    // The first sweep is a catalog crawl across every source — nothing on
    // screen needs it at launch, so it rides the last boot stagger tier.
    // Event-driven force checks (install/enable/disable) stay immediate.
    const cancelFirst = afterBoot(10000, () => void check(true));
    const interval = window.setInterval(() => void check(), CHECK_INTERVAL_MS);
    const onFocus = () => void check();
    window.addEventListener('focus', onFocus);
    let unlisten: (() => void) | undefined;
    listen('plugin://state-changed', () => void check(true)).then((u) => {
      if (alive) unlisten = u;
      else u();
    });

    return () => {
      alive = false;
      cancelFirst();
      window.clearInterval(interval);
      window.removeEventListener('focus', onFocus);
      unlisten?.();
    };
  }, []);

  return null;
}
