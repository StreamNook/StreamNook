import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DownloadCloud, Check, Loader2, Square, RefreshCw } from 'lucide-react';
import { SettingsSection, SettingsRow } from './_primitives';
import { inlineEmoteTier, refreshEmoteFileCache } from '../../services/emoteService';
import { Logger } from '../../utils/logger';

// Mirrors the Rust PrefetchProgress (serde serializes the snake_case fields as-is).
interface PrefetchProgress {
  phase: 'idle' | 'scanning' | 'planned' | 'downloading' | 'complete' | 'cancelled';
  channels_total: number;
  channels_done: number;
  current_channel: string | null;
  total_emotes: number;
  already_cached: number;
  to_download: number;
  downloaded: number;
  failed: number;
  estimated_bytes: number;
  seventv_unavailable: boolean;
  warning: string | null;
}

function formatSize(bytes: number): string {
  if (bytes <= 0) return '0 MB';
  const mb = bytes / (1024 * 1024);
  return mb < 1 ? `${Math.max(1, Math.round(bytes / 1024))} KB` : `${mb.toFixed(1)} MB`;
}

const EmotePrefetchSection = () => {
  const [progress, setProgress] = useState<PrefetchProgress | null>(null);

  // Re-sync to any running job + listen for live updates while this tab is open.
  useEffect(() => {
    let mounted = true;
    let unlistenProgress: (() => void) | undefined;
    let unlistenComplete: (() => void) | undefined;
    (async () => {
      try {
        const status = await invoke<PrefetchProgress>('emote_prefetch_status');
        if (mounted) setProgress(status);
      } catch (e) {
        Logger.warn('[EmotePrefetch] status failed:', e);
      }
      unlistenProgress = await listen<PrefetchProgress>('emote-prefetch-progress', (e) => {
        if (mounted) setProgress(e.payload);
      });
      unlistenComplete = await listen<PrefetchProgress>('emote-prefetch-complete', (e) => {
        if (mounted) setProgress(e.payload);
      });
    })();
    return () => {
      mounted = false;
      if (unlistenProgress) unlistenProgress();
      if (unlistenComplete) unlistenComplete();
    };
  }, []);

  // When a run finishes, merge the new files into the picker's in-memory disk
  // map so they render disk-first this session, not only on next launch.
  useEffect(() => {
    if (progress?.phase === 'complete') void refreshEmoteFileCache();
  }, [progress?.phase]);

  const scan = async () => {
    try { await invoke('emote_prefetch_plan', { tier: inlineEmoteTier() }); } catch (e) { Logger.warn('[EmotePrefetch] scan failed:', e); }
  };
  const startDownload = async () => {
    try { await invoke('emote_prefetch_start'); } catch (e) { Logger.warn('[EmotePrefetch] start failed:', e); }
  };
  const stop = async () => {
    try { await invoke('emote_prefetch_stop'); } catch (e) { Logger.warn('[EmotePrefetch] stop failed:', e); }
  };

  const phase = progress?.phase ?? 'idle';
  const scanPct = progress && progress.channels_total > 0
    ? Math.round((progress.channels_done / progress.channels_total) * 100)
    : 0;
  const dlPct = progress && progress.to_download > 0
    ? Math.round((progress.downloaded / progress.to_download) * 100)
    : 0;

  const primaryBtn = 'px-4 py-2 rounded-lg bg-accent/15 text-accent text-sm font-semibold hover:bg-accent/25 transition-all flex items-center justify-center gap-2';
  const ghostBtn = 'px-3 py-2 rounded-lg glass-button text-textSecondary hover:text-textPrimary text-sm transition-all flex items-center justify-center gap-2';

  return (
    <SettingsSection
      label="Emote Prefetch"
      description="Download every emote from the channels you follow ahead of time, so the emote menu opens instantly in any of their chats."
    >
      <SettingsRow
        title="Download emotes for every channel you follow"
        description="Scan your follows to see how many emotes are missing and how much space they need, then download them in the background while you do something else."
        help="1. Scan follows. 2. Check the count and size. 3. Download. Shared emotes are stored once and anything already cached is skipped, so a rerun only fetches what is new. Best started when you are away from your desk."
      >
        {progress?.warning && (
          <div className="mb-3 flex items-start gap-2 text-[12px] text-amber-400/90 bg-amber-400/10 rounded-lg px-3 py-2">
            <span aria-hidden>⚠</span>
            <span>{progress.warning}</span>
          </div>
        )}
        {/* IDLE */}
        {phase === 'idle' && (
          <button onClick={scan} className={primaryBtn}>
            <RefreshCw size={15} /> Scan follows
          </button>
        )}

        {/* SCANNING */}
        {phase === 'scanning' && (
          <div className="flex flex-col gap-2">
            <div className="flex items-center gap-2 text-[13px] text-textPrimary">
              <Loader2 size={14} className="animate-spin text-accent" /> Scanning your follows…
            </div>
            <div className="h-1.5 bg-surface rounded-full overflow-hidden">
              <div className="h-full rounded-full bg-accent transition-all duration-300" style={{ width: `${scanPct}%` }} />
            </div>
            <div className="flex justify-between text-[11px] text-textSecondary tabular-nums">
              <span>{progress?.channels_done ?? 0} / {progress?.channels_total ?? 0} channels</span>
              <span>{progress?.total_emotes ?? 0} emotes found</span>
            </div>
          </div>
        )}

        {/* PLANNED */}
        {phase === 'planned' && progress && (
          <div className="flex flex-col gap-3">
            {progress.to_download === 0 ? (
              <div className="flex items-center gap-2 text-[13px] text-textPrimary">
                <Check size={15} className="text-emerald-400" />
                All {progress.total_emotes} emotes across {progress.channels_total} channels are already cached.
              </div>
            ) : (
              <p className="text-[12px] text-textSecondary leading-relaxed">
                <span className="text-textPrimary font-medium">{progress.to_download}</span> emotes to download
                {' '}(~{formatSize(progress.estimated_bytes)}) across {progress.channels_total} channels.
                {' '}<span className="text-textPrimary font-medium">{progress.already_cached}</span> already cached.
              </p>
            )}
            <div className="flex gap-2">
              {progress.to_download > 0 && (
                <button onClick={startDownload} className={primaryBtn}>
                  <DownloadCloud size={15} /> Download {progress.to_download} emotes
                </button>
              )}
              <button onClick={scan} className={ghostBtn}><RefreshCw size={14} /> Rescan</button>
            </div>
          </div>
        )}

        {/* DOWNLOADING */}
        {phase === 'downloading' && progress && (
          <div className="flex flex-col gap-2">
            <div className="flex items-center gap-2 text-[13px] text-textPrimary">
              <Loader2 size={14} className="animate-spin text-accent" /> Downloading emotes…
            </div>
            <div className="h-1.5 bg-surface rounded-full overflow-hidden">
              <div className="h-full rounded-full bg-accent transition-all duration-300" style={{ width: `${dlPct}%` }} />
            </div>
            <div className="flex justify-between text-[11px] text-textSecondary tabular-nums">
              <span>{progress.downloaded} / {progress.to_download}</span>
              <span>{progress.failed > 0 ? `${progress.failed} failed · ` : ''}{dlPct}%</span>
            </div>
            <button onClick={stop} className={`${ghostBtn} mt-1 self-start`}><Square size={13} /> Stop</button>
          </div>
        )}

        {/* COMPLETE / CANCELLED */}
        {(phase === 'complete' || phase === 'cancelled') && progress && (
          <div className="flex flex-col gap-3">
            <div className="flex items-center gap-2 text-[13px] text-textPrimary">
              {phase === 'complete'
                ? <Check size={15} className="text-emerald-400" />
                : <Square size={14} className="text-textSecondary" />}
              {phase === 'complete' ? 'Prefetch complete.' : 'Stopped.'}
              {' '}Downloaded {progress.downloaded} emote{progress.downloaded === 1 ? '' : 's'}
              {progress.already_cached > 0 ? `, ${progress.already_cached} already cached` : ''}
              {progress.failed > 0 ? ` · ${progress.failed} failed` : ''}.
            </div>
            <button onClick={scan} className={`${ghostBtn} self-start`}><RefreshCw size={14} /> Scan again</button>
          </div>
        )}
      </SettingsRow>
    </SettingsSection>
  );
};

export default EmotePrefetchSection;
