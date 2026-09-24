import { X, ExternalLink, ChevronDown, Github, Heart, Check, Download, AlertCircle, RefreshCw } from 'lucide-react';
import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { ACCENT_BUTTON, ACCENT_FILL } from './ui/glazeButtons';
import { invoke } from '@tauri-apps/api/core';
import type { Changelog, ChangelogRelease } from '../types';
import { motion, AnimatePresence } from 'framer-motion';
import { ReleaseNotes } from './changelog/ReleaseNotes';
import { useUpdateCheck } from './changelog/useUpdateCheck';
import { Logger } from '../utils/logger';

// Compare versions ignoring a leading "v" (tags and props mix both forms).
const normalizeTag = (t: string) => t.replace(/^v/i, '').trim();

const formatShortDate = (iso: string | null): string => {
  if (!iso) return '';
  try {
    // A bare date (the offline fallback reads it off CHANGELOG.md) is a
    // calendar day, not UTC midnight, or it shows as the day before in the Americas.
    const d = /^\d{4}-\d{2}-\d{2}$/.test(iso) ? new Date(`${iso}T00:00:00`) : new Date(iso);
    return d.toLocaleDateString('en-US', {
      month: 'short',
      day: 'numeric',
      year: 'numeric',
    });
  } catch {
    return '';
  }
};

interface ChangelogOverlayProps {
  version: string;
  onClose: () => void;
}

const CHANGELOG_URL = 'https://github.com/winters27/StreamNook/blob/main/CHANGELOG.md';
const GITHUB_ISSUE_URL = 'https://github.com/winters27/StreamNook/issues/new';
const COMMUNITY_DISCORD_INVITE = 'https://discord.gg/2xvuF9TES7';

// Opens an external URL in the OS browser via Tauri's shell plugin (not the
// in-app WebView). Falls back to window.open if the plugin import fails.
const openExternal = async (url: string) => {
  try {
    const { open } = await import('@tauri-apps/plugin-shell');
    await open(url);
  } catch (err) {
    Logger.error('Failed to open external URL:', err);
    window.open(url, '_blank');
  }
};

// Official Discord brand mark, sized to match the lucide icons around it.
const DiscordIcon = ({ size = 14 }: { size?: number }) => (
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width={size} height={size} fill="currentColor" aria-hidden="true">
    <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.946 2.418-2.157 2.418z" />
  </svg>
);


// Sign-off + community links. Closes the notes as their last word, where
// someone who just read about a change is most likely to have something to say.
const communityLink =
  'glass-button-secondary inline-flex items-center gap-1.5 px-3 py-1.5 text-[12.5px] font-medium text-textSecondary hover:text-textPrimary';

const SignOff = () => (
  <div className="flex flex-col items-center text-center pt-2">
    <p className="text-[13px] text-textSecondary leading-snug">
      Run into a bug or have an idea? Reach out and let us know.
    </p>
    <div className="flex justify-center gap-2 mt-3">
      <button type="button" onClick={() => openExternal(GITHUB_ISSUE_URL)} className={communityLink}>
        <Github size={14} />
        GitHub
      </button>
      <button type="button" onClick={() => openExternal(COMMUNITY_DISCORD_INVITE)} className={communityLink}>
        <DiscordIcon size={14} />
        Discord
      </button>
    </div>
    <p className="flex items-center justify-center gap-1 mt-4 text-[11px] text-textMuted italic leading-snug">
      May your points always claim, your streams never buffer, and your drops always finish.
      <Heart size={11} className="inline-block text-accent shrink-0" fill="currentColor" />
    </p>
  </div>
);

const ChangelogOverlay = ({ version, onClose }: ChangelogOverlayProps) => {
  // Rust fetches, caches and parses every recent release (falling back to the
  // cache, then to this version's CHANGELOG.md section), so switching versions
  // is instant and nothing here parses anything.
  const [releases, setReleases] = useState<ChangelogRelease[] | null>(null);
  const [selectedTag, setSelectedTag] = useState<string>(() => normalizeTag(version));
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const switcherRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  // Set once the user picks a version from the switcher, so the auto-default
  // below stops overriding their choice.
  const userPickedRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    invoke<Changelog>('get_changelog', { version })
      .then((c) => {
        if (!cancelled) setReleases(c.releases);
      })
      .catch((err) => {
        Logger.error('Failed to load the changelog:', err);
        if (!cancelled) setError('Failed to load release notes');
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [version]);

  // Default the open changelog to what you just updated to (the `version` prop is
  // the installed version, i.e. the most recent release). If that exact tag isn't
  // in the fetched list, fall back to the newest release available. Re-evaluates
  // whenever the list changes — crucially when the fresh fetch replaces a stale
  // cache — so a stale cache can't pin the popup to an older release. Skips once
  // the user has manually picked a version from the switcher.
  useEffect(() => {
    if (userPickedRef.current || !releases || !releases.length) return;
    const wanted = normalizeTag(version);
    const hasWanted = releases.some((r) => r.version === wanted);
    setSelectedTag(hasWanted ? wanted : releases[0].version);
  }, [releases, version]);

  // Another version's notes start at their top, not at wherever the last one
  // was scrolled to.
  useEffect(() => {
    bodyRef.current?.scrollTo({ top: 0 });
  }, [selectedTag]);

  // Close the version menu on an outside click.
  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (switcherRef.current && !switcherRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [menuOpen]);

  const currentRelease = releases?.find((r) => r.version === selectedTag);
  const notes = currentRelease?.notes ?? null;
  const displayVersion = currentRelease?.version ?? normalizeTag(version);
  const hasSwitcher = !!releases && releases.length > 1;

  const publishedAt = formatShortDate(currentRelease?.published_at ?? null);
  const update = useUpdateCheck();
  const pending = update.pending;
  const pendingInList =
    !!pending && !!releases?.some((r) => r.version === normalizeTag(pending.latest));

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.18 }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/25"
    >
      {/* Background overlay - click to close */}
      <div className="absolute inset-0" onClick={onClose} />

      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 12 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.96, y: 12 }}
        transition={{ type: 'spring', stiffness: 380, damping: 30 }}
        className="glass-modal relative z-10 w-[580px] max-w-[94vw] h-[780px] max-h-[90vh] flex flex-col"
      >
        <button
          onClick={onClose}
          aria-label="Close"
          className="absolute top-3.5 right-3.5 z-10 p-1.5 text-textMuted hover:text-textPrimary hover:bg-white/[0.06] rounded-full transition-colors duration-150"
        >
          <X size={16} />
        </button>

        {/* Header. The version is the subtitle and doubles as the switcher, so
            what you are reading and the way to read another are one control. */}
        <div className="flex flex-col items-center px-10 pt-8 pb-5">
          <h2 className="text-[24px] font-bold text-textPrimary tracking-tight">What's New</h2>
          {hasSwitcher ? (
            <div className="relative mt-1.5" ref={switcherRef}>
              <button
                onClick={() => setMenuOpen((o) => !o)}
                aria-haspopup="listbox"
                aria-expanded={menuOpen}
                className="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[13px] text-textMuted hover:text-textPrimary hover:bg-white/[0.05] transition-colors"
              >
                <span className="font-medium text-textSecondary">{displayVersion}</span>
                {publishedAt && <span>· {publishedAt}</span>}
                <ChevronDown
                  size={13}
                  className={`transition-transform duration-200 ${menuOpen ? 'rotate-180' : ''}`}
                />
              </button>
              <AnimatePresence>
                {menuOpen && (
                  <motion.div
                    initial={{ opacity: 0, y: -4 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -4 }}
                    transition={{ duration: 0.14 }}
                    className="sn-popover absolute top-full left-1/2 -ml-[108px] mt-2 w-[216px] z-20 p-1.5"
                    // Near-opaque: this menu opens over the notes, and a menu is
                    // something you read, so the text behind must not show through.
                    style={{ '--sn-popover-tint': '96%' } as CSSProperties}
                  >
                    <div role="listbox" className="max-h-72 overflow-y-auto custom-scrollbar">
                      {releases!.map((r) => {
                        const tag = r.version;
                        const active = tag === selectedTag;
                        return (
                          <button
                            key={tag}
                            role="option"
                            aria-selected={active}
                            onClick={() => {
                              userPickedRef.current = true;
                              setSelectedTag(tag);
                              setMenuOpen(false);
                            }}
                            className={`w-full flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors ${
                              active
                                ? 'bg-white/[0.08] text-textPrimary'
                                : 'text-textSecondary hover:bg-white/[0.05] hover:text-textPrimary'
                            }`}
                          >
                            <span className="w-3.5 shrink-0">
                              {active && <Check size={13} className="text-accent" />}
                            </span>
                            <span className="flex-1 text-[13px] font-medium">{tag}</span>
                            <span className="text-[11px] text-textMuted">
                              {formatShortDate(r.published_at)}
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  </motion.div>
                )}
              </AnimatePresence>
            </div>
          ) : (
            <div className="mt-1.5 px-2.5 py-1 text-[13px] text-textMuted">
              <span className="font-medium text-textSecondary">{displayVersion}</span>
              {publishedAt && <span> · {publishedAt}</span>}
            </div>
          )}
        </div>

        {/* Update. Only drawn when there is something to act on: a new version
            to install, or a check that failed. Sits above the notes so the way
            to get a release is next to the story of what is in it. */}
        {(pending || update.failure) && (
          <div className="px-7 pb-3">
            {pending ? (
              <div className="glaze-inset rounded-xl bg-white/[0.03] flex items-center gap-3 pl-4 pr-3 py-3">
                <Download size={18} className="text-accent shrink-0" />
                <div className="min-w-0 flex-1">
                  <div className="text-[14px] font-semibold text-textPrimary leading-snug">
                    StreamNook {normalizeTag(pending.latest)} is ready
                  </div>
                  <div className="mt-0.5 text-[12.5px] text-textMuted leading-snug">
                    You're on {normalizeTag(pending.current)}
                    {pending.size && <> · {pending.size}</>}
                    {pending.behind && pending.behind > 1 && (
                      <> · at least {pending.behind} releases behind</>
                    )}
                    {pendingInList && selectedTag !== normalizeTag(pending.latest) && (
                      <>
                        {' · '}
                        <button
                          type="button"
                          onClick={() => {
                            userPickedRef.current = true;
                            setSelectedTag(normalizeTag(pending.latest));
                          }}
                          className="text-textSecondary underline-offset-2 hover:text-textPrimary hover:underline"
                        >
                          See what's in it
                        </button>
                      </>
                    )}
                  </div>
                </div>
                <button
                  type="button"
                  onClick={update.install}
                  disabled={update.starting}
                  className={`${ACCENT_BUTTON} shrink-0 px-5 py-1.5`}
                  style={ACCENT_FILL}
                >
                  {update.starting ? 'Starting…' : 'Install'}
                </button>
              </div>
            ) : (
              <div className="glaze-inset rounded-xl bg-white/[0.03] flex items-center gap-3 pl-4 pr-3 py-3">
                <AlertCircle size={18} className="text-amber-400 shrink-0" />
                <p className="min-w-0 flex-1 text-[12.5px] text-textSecondary leading-snug">
                  {update.failure}
                </p>
                <button
                  type="button"
                  onClick={update.runCheck}
                  disabled={update.checking}
                  className="glass-button-secondary shrink-0 px-3 py-1.5 text-[12.5px] font-medium text-textSecondary hover:text-textPrimary"
                >
                  {update.checking ? 'Checking…' : 'Try again'}
                </button>
              </div>
            )}
          </div>
        )}

        {/* Body */}
        <div
          ref={bodyRef}
          className="flex-1 overflow-y-auto custom-scrollbar px-7 pt-2 pb-8"
          // Notes fade out under the header instead of being cut by a hard edge.
          style={{
            maskImage: 'linear-gradient(to bottom, transparent, #000 14px)',
            WebkitMaskImage: 'linear-gradient(to bottom, transparent, #000 14px)',
          }}
        >
          {isLoading && !notes ? (
            <div className="flex items-center justify-center py-10">
              <div className="animate-spin rounded-full h-7 w-7 border-b-2 border-accent" />
            </div>
          ) : error && !notes ? (
            <div className="text-center py-10">
              <p className="text-sm text-textSecondary">{error}</p>
            </div>
          ) : notes ? (
            <ReleaseNotes nodes={notes} />
          ) : (
            <div className="text-center py-10">
              <p className="text-sm text-textSecondary">No release notes available</p>
            </div>
          )}

          <div className="mt-10">
            <SignOff />
          </div>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between gap-3 px-6 py-4 border-t border-white/[0.06]">
          <div className="flex items-center gap-4">
            <button
              type="button"
              onClick={() => openExternal(CHANGELOG_URL)}
              className="inline-flex items-center gap-1.5 text-[12.5px] text-textMuted hover:text-textPrimary transition-colors"
            >
              <ExternalLink size={13} />
              Full changelog
            </button>
            {/* Once an update is known, Install above is the action; a second
                check here would only repeat it. */}
            {!pending && (
              <button
                type="button"
                onClick={update.runCheck}
                disabled={update.checking}
                className="inline-flex items-center gap-1.5 text-[12.5px] text-textMuted hover:text-textPrimary disabled:hover:text-textMuted transition-colors"
              >
                {update.upToDate && !update.checking ? (
                  <>
                    <Check size={13} className="text-emerald-400" />
                    Up to date
                  </>
                ) : (
                  <>
                    <RefreshCw size={13} className={update.checking ? 'animate-spin' : ''} />
                    {update.checking ? 'Checking…' : 'Check for updates'}
                  </>
                )}
              </button>
            )}
          </div>
          <button onClick={onClose} className={`${ACCENT_BUTTON} px-7 py-2`} style={ACCENT_FILL}>
            Continue
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
};

export default ChangelogOverlay;
