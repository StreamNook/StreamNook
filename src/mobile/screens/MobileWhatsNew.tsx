// What's New, for the Android build.
//
// The desktop panel lists GitHub releases from winters27/StreamNook, which are
// DESKTOP releases: their notes describe desktop fixes, and their version
// numbers are the 8.x line the phone does not follow. Showing them here told a
// phone user about changes that never shipped to them.
//
// The mobile changelog is the `notes` field of the Android update manifest,
// which is written next to the APK it describes. That makes the changelog and
// the build it belongs to physically impossible to get out of step.
//
// Limitation worth knowing: the manifest only ever describes the CURRENT
// release, so this shows one entry rather than a history. A real archive would
// need a separate object in R2 that accumulates past releases; that is worth
// doing once there are enough Android releases for a history to mean anything.
import React, { useEffect, useMemo, useState } from 'react';
import {
  BatteryCharging,
  BellSimple,
  ChatCircleText,
  CheckCircle,
  DownloadSimple,
  LinkSimple,
  Path,
  PlayCircle,
  SpeakerHigh,
  Sparkle,
  UserCircle,
  Wrench,
} from 'phosphor-react';
import { getAppVersion } from '../updateCheck';
import { Logger } from '../../utils/logger';

const MANIFEST_URL = 'https://streamnook.app/api/v1/update-android';

interface Release {
  version: string;
  notes?: string;
  published_at?: string;
}

/**
 * One change, as the notes are actually written.
 *
 * Every paragraph in the manifest follows the same shape: a headline sentence,
 * then the detail. "Much easier on the battery. The app now matches video
 * quality to what your screen can actually show..." Splitting on that gives a
 * real title and body to lay out, instead of one preformatted block where
 * every line carries the same weight and nothing is scannable.
 */
interface Change {
  title: string;
  body: string;
}

function parseChanges(notes: string): Change[] {
  return notes
    .split(/\n\s*\n/)
    .map((p) => p.trim())
    .filter(Boolean)
    .map((para) => {
      // The whitespace after the stop is load-bearing: without it "0.1.9" and
      // "e.g." split a sentence in half.
      const m = para.match(/^(.+?[.!?])\s+([\s\S]+)$/);
      if (!m) return { title: para.replace(/\.$/, ''), body: '' };
      return { title: m[1].replace(/\.$/, ''), body: m[2].trim() };
    });
}

/**
 * An icon for a change, chosen from what it talks about.
 *
 * Deliberately a small set with an honest fallback. A confidently wrong icon
 * reads worse than a neutral one, so anything unmatched gets the spark rather
 * than a guess.
 */
function iconFor(title: string): React.ElementType {
  const t = title.toLowerCase();
  if (/(audio|listen|lock|media control|background|picture-in-picture)/.test(t)) return SpeakerHigh;
  if (/(batter|cooler|hot|power|data|performance|quality)/.test(t)) return BatteryCharging;
  // Playback words are some of the commonest things a release note is about,
  // and the first real changelog through here matched none of the original
  // rules: every entry fell back to the spark, which makes the whole layout
  // look like it is not trying.
  if (/(pause|paused|play|player|playback|stream|video|buffer|latency)/.test(t)) return PlayCircle;
  if (/(profile|badge|paint|identity|avatar|cosmetic)/.test(t)) return UserCircle;
  if (/(link|clip)/.test(t)) return LinkSimple;
  if (/(chat|reply|message|emote)/.test(t)) return ChatCircleText;
  if (/(notification|alert)/.test(t)) return BellSimple;
  if (/(fix|bug|no longer|stuck|crash)/.test(t)) return Wrench;
  if (/(emulator|build|install|update)/.test(t)) return Path;
  return Sparkle;
}

/** Designed skeleton rather than the word "Loading". */
const Skeleton: React.FC = () => (
  <div className="space-y-4" aria-busy="true" aria-label="Loading release notes">
    <div className="flex items-center gap-3">
      <div className="h-11 w-11 rounded-xl bg-surface animate-pulse shrink-0" />
      <div className="flex-1 space-y-2">
        <div className="h-5 w-20 rounded bg-surface animate-pulse" />
        <div className="h-3 w-36 rounded bg-surface animate-pulse" />
      </div>
    </div>
    <div className="space-y-2">
      {[0, 1, 2].map((i) => (
        <div key={i} className="glass-tile rounded-xl px-[13px] py-3 flex gap-3">
          <div className="h-8 w-8 rounded-lg bg-surface animate-pulse shrink-0" />
          <div className="flex-1 space-y-2 pt-0.5">
            <div className="h-3.5 w-2/5 rounded bg-surface animate-pulse" />
            <div className="h-3 w-full rounded bg-surface animate-pulse" />
            <div className="h-3 w-4/5 rounded bg-surface animate-pulse" />
          </div>
        </div>
      ))}
    </div>
  </div>
);

export const MobileWhatsNew: React.FC = () => {
  const [release, setRelease] = useState<Release | null>(null);
  const [running, setRunning] = useState<string | null>(null);
  const [state, setState] = useState<'loading' | 'ready' | 'unavailable'>('loading');

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [ver, res] = await Promise.all([
          getAppVersion(),
          fetch(MANIFEST_URL, { cache: 'no-store' }),
        ]);
        if (cancelled) return;
        setRunning(ver);
        // 503 is the documented "nothing published yet" state.
        if (!res.ok) {
          setState('unavailable');
          return;
        }
        setRelease((await res.json()) as Release);
        setState('ready');
      } catch (err) {
        Logger.warn('[WhatsNew] could not load the Android release notes:', err);
        if (!cancelled) setState('unavailable');
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Hoisted so the dependency is the string itself. Depending on
  // `release?.notes` inline reads as a dependency on `release` to the React
  // Compiler, which then refuses to preserve the memo at all.
  const notes = release?.notes;
  const changes = useMemo(() => (notes ? parseChanges(notes) : []), [notes]);

  if (state === 'loading') return <Skeleton />;

  if (state === 'unavailable' || !release) {
    return (
      <div className="flex flex-col items-center text-center py-12 px-6">
        <div className="h-14 w-14 rounded-2xl bg-surface flex items-center justify-center mb-3">
          <Sparkle size={24} className="text-textMuted" />
        </div>
        <div className="text-[15px] font-semibold text-textPrimary">No release notes right now</div>
        <div className="mt-1 text-[13px] text-textMuted max-w-[260px] leading-relaxed">
          These load from the update service, so this usually just means no connection.
        </div>
        {running && (
          <div className="mt-4 text-[12.5px] text-textMuted">
            You are on <span className="text-textSecondary font-medium">{running}</span>
          </div>
        )}
      </div>
    );
  }

  const date = release.published_at
    ? new Date(release.published_at).toLocaleDateString(undefined, {
        year: 'numeric',
        month: 'long',
        day: 'numeric',
      })
    : null;
  // The published version is not necessarily the one running: someone can be a
  // release behind. Saying so is more useful than implying they match.
  const isRunning = running === release.version;

  return (
    <div className="space-y-4 pb-2">
      {/* Version hero. The number is the headline so it gets the size, and the
          status line under it answers the only question this screen raises:
          is this the build I am actually holding? */}
      <div className="flex items-center gap-3">
        <div className="h-11 w-11 rounded-xl bg-surface flex items-center justify-center shrink-0">
          <Sparkle size={22} weight="fill" className="text-accent" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2 min-w-0">
            <span className="text-[26px] leading-none font-bold text-textPrimary tracking-tight">
              {release.version}
            </span>
            {date && <span className="text-[12.5px] text-textMuted truncate">{date}</span>}
          </div>
          <div className="mt-1.5">
            {isRunning ? (
              <span className="inline-flex items-center gap-1.5 text-[12.5px] text-success">
                <CheckCircle size={14} weight="fill" />
                You are on this version
              </span>
            ) : (
              <span className="inline-flex items-center gap-1.5 text-[12.5px] text-accent">
                <DownloadSimple size={14} weight="bold" />
                Update available
                {running && <span className="text-textMuted">· you are on {running}</span>}
              </span>
            )}
          </div>
        </div>
      </div>

      {changes.length > 0 ? (
        // Density is tuned, not guessed: measured on device, `leading-relaxed`
        // body copy made one card 330px tall and fit 2.5 of them on screen,
        // which reads as a document rather than a changelog. At 1.5 line height
        // a card is 185px and the whole release is nearly scannable at once.
        <div className="space-y-2">
          {changes.map((c, i) => {
            const Icon = iconFor(c.title);
            return (
              <div key={i} className="glass-tile rounded-xl px-[13px] py-3 flex gap-3">
                <div className="h-8 w-8 rounded-lg bg-surface flex items-center justify-center shrink-0 mt-0.5">
                  <Icon size={17} className="text-accent" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="text-[14.5px] font-semibold text-textPrimary leading-snug">
                    {c.title}
                  </div>
                  {c.body && (
                    <div className="mt-[3px] text-[13px] leading-[1.5] text-textSecondary">
                      {c.body}
                    </div>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <div className="text-[14px] text-textMuted">No notes for this release.</div>
      )}
    </div>
  );
};

export default MobileWhatsNew;
