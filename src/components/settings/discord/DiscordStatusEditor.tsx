// The Discord Status editor: controls on the left, the card Discord will show
// on the right. Rust builds the card (`discord_presence_preview`) from the
// draft, so the preview is exactly what Discord gets. Edits save field by field
// through the shared committer (discordDraft.ts).

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { motion } from 'framer-motion';
import { X } from 'lucide-react';
import { useAppStore } from '../../../stores/AppStore';
import type { DiscordPresenceSettings } from '../../../types';
import { ResetArrow, SegmentedSelect, SettingsRow, SettingsSection } from '../_primitives';
import { SubControl, SubControls } from '../../plugins/settingsPageKit';
import { Toggle } from '../../ui/Toggle';
import { Tooltip } from '../../ui/Tooltip';
import { DiscordGlyph } from '../../ui/DiscordGlyph';
import { useSettingDefault, useSettingReset } from '../settingReset';
import { flashSettingRow } from '../settingsNavigation';
import { setSettingsFields } from '../../../utils/settingsFields';
import { draftOf, presenceCommitter, reconcile, type Pending, type PresenceField } from './discordDraft';
import { DiscordStatusPreview, type PreviewModel, type PreviewTab } from './DiscordStatusPreview';

const CHANGED_EVENT = 'discord-presence://changed';
const COMMIT_IDLE_MS = 500;
const PREVIEW_DEBOUNCE_MS = 80;
const CARD_WIDTH = 340;

const TABS: { id: PreviewTab; label: string }[] = [
  { id: 'live', label: 'Live' },
  { id: 'replay', label: 'VOD & clips' },
  { id: 'grid', label: 'MultiNook' },
  { id: 'offline', label: 'Offline chat' },
  { id: 'browsing', label: 'Browsing' },
];

/** The two lines each tab edits. */
const LINES: Record<Exclude<PreviewTab, 'browsing'>, [PresenceField, PresenceField]> = {
  live: ['watching_top', 'watching_bottom'],
  replay: ['replay_top', 'replay_bottom'],
  grid: ['grid_top', 'grid_bottom'],
  offline: ['offline_top', 'offline_bottom'],
};

/** Marks a setting only StreamNook subscribers can change. Locked, it opens
 *  the subscribe page (the same link as Profile and Badges). */
function SubscriberPill({ locked }: { locked: boolean }) {
  const login = useAppStore((s) => s.currentUser?.login || s.currentUser?.username);
  const pill = 'rounded-full bg-accent/15 px-1.5 py-[1px] text-[10px] font-semibold uppercase tracking-wide text-accent';
  if (!locked) return <span className={pill}>Subscriber</span>;
  const subscribe = () => {
    void (async () => {
      try {
        const { open } = await import('@tauri-apps/plugin-shell');
        const handle = login ? `&handle=${encodeURIComponent(login)}` : '';
        await open(`https://streamnook.app/support?tier=subscriber${handle}`);
      } catch {
        /* opening the browser is best-effort */
      }
    })();
  };
  return (
    <Tooltip content="Subscribe on streamnook.app" describe>
      <button type="button" onClick={subscribe} className={`${pill} transition-colors hover:bg-accent/25`}>
        Subscriber
      </button>
    </Tooltip>
  );
}

/** The tab a search hit's row lives on (rows that show only on one tab). */
function tabForRow(row: string | undefined): PreviewTab | null {
  if (row === 'Show' || row === 'Text') return 'browsing';
  if (row === 'Top line' || row === 'Bottom line') return 'live';
  return null;
}

/** A text field for a status line, with its tokens as chips that insert at the caret. */
function TemplateInput({
  value,
  onChange,
  tokens,
  placeholder,
  ariaLabel,
  max = 128,
}: {
  value: string;
  onChange: (next: string) => void;
  tokens: { token: string; value: string }[];
  placeholder?: string;
  ariaLabel: string;
  max?: number;
}) {
  const ref = useRef<HTMLInputElement>(null);
  const insert = (token: string) => {
    const el = ref.current;
    const at = el?.selectionStart ?? value.length;
    const end = el?.selectionEnd ?? at;
    const next = (value.slice(0, at) + token + value.slice(end)).slice(0, max);
    onChange(next);
    requestAnimationFrame(() => {
      el?.focus();
      const caret = Math.min(at + token.length, max);
      el?.setSelectionRange(caret, caret);
    });
  };
  return (
    <div className="space-y-2">
      <input
        ref={ref}
        value={value}
        maxLength={max}
        placeholder={placeholder}
        aria-label={ariaLabel}
        onChange={(e) => onChange(e.target.value)}
        className="glass-input w-full rounded-lg px-3 py-2 text-[13px] text-textPrimary"
      />
      {tokens.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {tokens.map((t) => (
            <Tooltip key={t.token} content={t.value ? `Now: ${t.value}` : ''} disabled={!t.value} describe>
              <button
                type="button"
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => insert(t.token)}
                className="rounded-md bg-white/[0.05] px-2 py-0.5 font-mono text-[11px] text-textSecondary transition-colors hover:bg-white/[0.1] hover:text-textPrimary"
              >
                {t.token}
              </button>
            </Tooltip>
          ))}
        </div>
      )}
    </div>
  );
}

export default function DiscordStatusEditor({ focusRow, onClose }: { focusRow?: string; onClose: () => void }) {
  const settings = useAppStore((s) => s.settings);
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
  const canonical = settings.discord_presence;
  const defaults = useSettingDefault('discord_presence') as DiscordPresenceSettings | undefined;
  const resetSwitch = useSettingReset()('discord_rpc_enabled');

  const [pending, setPending] = useState<Pending>({});
  const pendingRef = useRef<Pending>({});
  useLayoutEffect(() => {
    pendingRef.current = pending;
  }, [pending]);
  // A pending field leaves once the saved settings show it (adjusted during
  // render when the saved style changes, not in an effect).
  const [seenCanonical, setSeenCanonical] = useState(canonical);
  if (seenCanonical !== canonical) {
    setSeenCanonical(canonical);
    setPending((p) => reconcile(canonical, p));
  }

  const [hover, setHover] = useState<Pending | null>(null);
  const [tab, setTab] = useState<PreviewTab | null>(() => tabForRow(focusRow));
  const [model, setModel] = useState<PreviewModel | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const [cardScale, setCardScale] = useState(1);
  const commitTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const previewSeq = useRef(0);
  const panelRef = useRef<HTMLDivElement>(null);
  const leftRef = useRef<HTMLDivElement>(null);
  const rightRef = useRef<HTMLDivElement>(null);
  const downOnScrim = useRef(false);

  const draft = useMemo(() => draftOf(canonical, pending), [canonical, pending]);
  const previewStyle = useMemo(() => (hover ? { ...draft, ...hover } : draft), [draft, hover]);
  const commitStatus = useSyncExternalStore(presenceCommitter.subscribe, presenceCommitter.status);

  const flush = useCallback(() => {
    if (commitTimer.current) {
      clearTimeout(commitTimer.current);
      commitTimer.current = null;
    }
    presenceCommitter.queue(pendingRef.current);
    presenceCommitter.flush();
  }, []);

  const change = useCallback(<K extends PresenceField>(field: K, value: DiscordPresenceSettings[K]) => {
    setPending((p) => ({ ...p, [field]: value }));
    if (commitTimer.current) clearTimeout(commitTimer.current);
    commitTimer.current = setTimeout(() => {
      commitTimer.current = null;
      presenceCommitter.queue(pendingRef.current);
    }, COMMIT_IDLE_MS);
  }, []);

  const close = useCallback(() => {
    flush();
    onClose();
  }, [flush, onClose]);

  // Escape closes this editor only, never the Settings dialog under it.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopImmediatePropagation();
      close();
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('pagehide', flush);
    window.addEventListener('beforeunload', flush);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('pagehide', flush);
      window.removeEventListener('beforeunload', flush);
    };
  }, [close, flush]);

  // The editor can also go away without its close button (Settings closed,
  // the window gone): its last edits are sent first.
  useEffect(() => () => flush(), [flush]);

  // The dialog takes focus, so Tab stays in it rather than in Settings behind.
  useLayoutEffect(() => {
    panelRef.current?.focus();
  }, []);

  // The card, rebuilt by Rust for the draft (and a hovered choice).
  const styleKey = JSON.stringify(previewStyle);
  useEffect(() => {
    const seq = ++previewSeq.current;
    const timer = setTimeout(() => {
      invoke<PreviewModel>('discord_presence_preview', { style: previewStyle, scene: tab ?? 'live' })
        .then((m) => {
          if (seq !== previewSeq.current) return;
          // First open: show the tab Discord shows now, without flashing another.
          if (tab === null && m.shown !== 'live') {
            setTab(m.shown);
            return;
          }
          if (tab === null) setTab('live');
          setModel(m);
        })
        .catch(() => {});
    }, PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(timer);
    // styleKey stands for previewStyle's contents.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [styleKey, tab, refreshTick]);

  useEffect(() => {
    let stop: (() => void) | undefined;
    let live = true;
    void listen(CHANGED_EVENT, () => setRefreshTick((n) => n + 1)).then((u) => {
      if (live) stop = u;
      else u();
    });
    const ticker = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      live = false;
      stop?.();
      clearInterval(ticker);
    };
  }, []);

  // The card shrinks with a narrow window instead of the panes stacking.
  useLayoutEffect(() => {
    const el = rightRef.current;
    if (!el) return;
    const fit = () => setCardScale(Math.min(1, Math.max(0.6, (el.clientWidth - 40) / CARD_WIDTH)));
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Opened from a search hit: bring that row (or the row it hides under) into view.
  useEffect(() => {
    if (!focusRow) return;
    const id = requestAnimationFrame(() => {
      const find = (t: string) => leftRef.current?.querySelector<HTMLElement>(`[data-setting-row="${CSS.escape(t)}"]`) ?? null;
      const el = find(focusRow) ?? (focusRow === 'Text' ? find('Show') : focusRow === 'Label' ? find('Watch button') : null);
      if (!el) return;
      el.scrollIntoView({ block: 'center' });
      flashSettingRow(el);
    });
    return () => cancelAnimationFrame(id);
  }, [focusRow]);

  const reset = <K extends PresenceField>(field: K) =>
    defaults && draft[field] !== defaults[field] ? () => change(field, defaults[field]) : undefined;
  const preview = (field: PresenceField) => (value: string | null) =>
    setHover(value === null ? null : ({ [field]: value } as Pending));
  // A preset carries only the fields it sets; anything else stays the viewer's.
  const presetActive = (fields: Record<string, unknown>) =>
    Object.keys(fields).every((k) => fields[k] === (draft as unknown as Record<string, unknown>)[k]);
  const applyPreset = (fields: Record<string, unknown>) => {
    for (const key of Object.keys(fields) as PresenceField[]) {
      if (key in draft && fields[key] !== draft[key]) {
        change(key, fields[key] as DiscordPresenceSettings[typeof key]);
      }
    }
  };

  const shownTab = tab ?? 'live';
  const enabled = settings.discord_rpc_enabled;
  // Rust decides who subscribes (services/membership_status.rs).
  const member = model?.member ?? false;
  const status =
    commitStatus === 'retrying'
      ? "Couldn't save, retrying"
      : !model
        ? ''
        : model.connection === 'off'
          ? 'Discord Rich Presence is off'
          : model.connection === 'waiting'
            ? 'Open Discord and this shows up on your profile'
            : model.connection === 'connecting'
              ? 'Updating your Discord status'
              : model.user
                ? `Showing on Discord as ${model.user.name}`
                : 'Showing on Discord';
  const lines = shownTab === 'browsing' ? null : LINES[shownTab];

  return createPortal(
    <motion.div
      className="fixed inset-0 z-[calc(var(--sn-z-over-player-lift,0)_+_60)] flex items-center justify-center bg-black/50 p-4"
      initial={motionOff ? false : { opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={motionOff ? undefined : { opacity: 0 }}
      transition={{ duration: 0.16, ease: 'easeOut' }}
      onMouseDown={(e) => {
        downOnScrim.current = e.target === e.currentTarget;
      }}
      onClick={(e) => {
        // Only a press that began and ended on the scrim closes, and it stops
        // here: the Settings dialog under it never hears it.
        e.stopPropagation();
        if (downOnScrim.current && e.target === e.currentTarget) close();
        downOnScrim.current = false;
      }}
    >
      <motion.div
        ref={panelRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label="Discord status"
        initial={motionOff ? false : { opacity: 0, scale: 0.98 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={motionOff ? undefined : { opacity: 0, scale: 0.98 }}
        transition={{ duration: 0.16, ease: 'easeOut' }}
        className="liquid-glass-panel flex flex-col overflow-hidden rounded-2xl outline-none"
        style={{ width: 'min(980px, 94vw)', height: 'min(680px, 90vh)' }}
      >
        <div className="flex h-14 flex-shrink-0 items-center gap-3 border-b border-white/[0.06] px-5">
          <DiscordGlyph size={20} className="text-[#5865F2]" />
          <div className="flex-1 text-[15px] font-semibold text-textPrimary">Discord Status</div>
          <span className="text-[12px] text-textSecondary">
            Show on Discord
            {resetSwitch && <ResetArrow onReset={resetSwitch} />}
          </span>
          <Toggle
            enabled={enabled}
            ariaLabel="Show on Discord"
            // One field, so a style write landing meanwhile is never put back by a stale copy.
            onChange={() => void setSettingsFields({ discord_rpc_enabled: !enabled }).catch(() => {})}
          />
          <button
            type="button"
            onClick={close}
            aria-label="Close"
            className="ml-1 rounded-lg p-1.5 text-textMuted transition-colors hover:bg-white/[0.06] hover:text-textPrimary"
          >
            <X size={16} />
          </button>
        </div>

        <div className="flex min-h-0 flex-1">
          <div ref={leftRef} className="custom-scrollbar w-[48%] min-w-0 space-y-6 overflow-y-auto px-5 py-5">
            <SettingsSection label="Presets" bare>
              <div className="grid grid-cols-3 gap-2">
                {(model?.presets ?? []).map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    onClick={() => applyPreset(p.fields)}
                    onMouseEnter={() => setHover(p.fields as Pending)}
                    onMouseLeave={() => setHover(null)}
                    onFocus={() => setHover(p.fields as Pending)}
                    onBlur={() => setHover(null)}
                    className={`glass-tile rounded-xl px-3 py-2.5 text-left transition-colors ${
                      presetActive(p.fields) ? 'ring-1 ring-accent/60' : ''
                    }`}
                  >
                    <div className="text-[13px] font-semibold text-textPrimary">{p.label}</div>
                    <div className="mt-0.5 text-[11px] leading-snug text-textSecondary">{p.blurb}</div>
                  </button>
                ))}
              </div>
            </SettingsSection>

            <SettingsSection label="Pictures">
              <SettingsRow
                title="Big picture"
                description="The large image on your status."
                onReset={reset('large_image')}
                control={
                  <SegmentedSelect
                    value={draft.large_image}
                    onChange={(v) => change('large_image', v)}
                    onPreview={preview('large_image')}
                    options={[
                      { value: 'category', label: 'Category' },
                      { value: 'streamer', label: 'Streamer' },
                      { value: 'streamnook', label: 'StreamNook' },
                    ]}
                  />
                }
              />
              <SettingsRow
                title="Small badge"
                description="The small circle on the big picture's corner."
                onReset={reset('small_image')}
                control={
                  <SegmentedSelect
                    value={draft.small_image}
                    onChange={(v) => change('small_image', v)}
                    onPreview={preview('small_image')}
                    options={[
                      { value: 'platform', label: 'Platform' },
                      { value: 'streamer', label: 'Streamer' },
                      { value: 'category', label: 'Category' },
                      { value: 'none', label: 'None' },
                    ]}
                  />
                }
              />
            </SettingsSection>

            <SettingsSection label="Text" description={`For ${TABS.find((t) => t.id === shownTab)?.label ?? 'Live'}, the tab in the preview.`}>
              {lines ? (
                <>
                  <SettingsRow title="Top line" onReset={reset(lines[0])}>
                    <TemplateInput
                      value={String(draft[lines[0]])}
                      onChange={(v) => change(lines[0], v)}
                      tokens={model?.tokens ?? []}
                      ariaLabel="Top line"
                    />
                  </SettingsRow>
                  <SettingsRow title="Bottom line" onReset={reset(lines[1])}>
                    <TemplateInput
                      value={String(draft[lines[1]])}
                      onChange={(v) => change(lines[1], v)}
                      tokens={model?.tokens ?? []}
                      placeholder="Leave empty for one line"
                      ariaLabel="Bottom line"
                    />
                  </SettingsRow>
                </>
              ) : (
                <SettingsRow
                  title="Show"
                  description="Your status while you look for something to watch."
                  onReset={reset('idle_mode')}
                  control={
                    <SegmentedSelect
                      value={draft.idle_mode}
                      onChange={(v) => change('idle_mode', v)}
                      onPreview={preview('idle_mode')}
                      options={[
                        { value: 'phrases', label: 'Fun phrases' },
                        { value: 'custom', label: 'Custom' },
                        { value: 'hidden', label: 'Nothing' },
                      ]}
                    />
                  }
                >
                  {draft.idle_mode === 'custom' && (
                    <SubControls>
                      <SubControl
                        title="Text"
                        stacked
                        onReset={reset('idle_text')}
                        control={
                          <TemplateInput
                            value={draft.idle_text}
                            onChange={(v) => change('idle_text', v)}
                            tokens={[]}
                            ariaLabel="Browsing text"
                          />
                        }
                      />
                    </SubControls>
                  )}
                </SettingsRow>
              )}
            </SettingsSection>

            <SettingsSection label="Status">
              <SettingsRow
                title="Activity"
                description="The word Discord puts before StreamNook."
                onReset={reset('activity_type')}
                control={
                  <SegmentedSelect
                    value={draft.activity_type}
                    onChange={(v) => change('activity_type', v)}
                    onPreview={preview('activity_type')}
                    options={[
                      { value: 'watching', label: 'Watching' },
                      { value: 'playing', label: 'Playing' },
                      { value: 'listening', label: 'Listening' },
                    ]}
                  />
                }
              />
              <SettingsRow
                title="Shown as"
                titleBadge={<SubscriberPill locked={!member} />}
                description={
                  member
                    ? 'The name after the activity word. Twitch and YouTube get their mark in Discord too.'
                    : 'Subscribers can show the platform instead, like "Watching Twitch".'
                }
                onReset={member ? reset('app_name') : undefined}
                control={
                  // Locked: only the control dims, so the Subscriber link stays clickable.
                  <div className={member ? undefined : 'pointer-events-none opacity-50'} aria-disabled={!member}>
                    <SegmentedSelect
                      value={member ? draft.app_name : 'streamnook'}
                      onChange={(v) => change('app_name', v)}
                      onPreview={member ? preview('app_name') : undefined}
                      options={[
                        { value: 'streamnook', label: 'StreamNook' },
                        { value: 'platform', label: 'The platform' },
                      ]}
                    />
                  </div>
                }
              />
              <SettingsRow
                title="Member list"
                description="The line under your name in a server's member list."
                onReset={reset('status_display')}
                control={
                  <SegmentedSelect
                    value={draft.status_display}
                    onChange={(v) => change('status_display', v)}
                    onPreview={preview('status_display')}
                    options={[
                      { value: 'app', label: member && draft.app_name === 'platform' ? 'The platform' : 'StreamNook' },
                      { value: 'top', label: 'Top line' },
                      { value: 'bottom', label: 'Bottom line' },
                    ]}
                  />
                }
              />
              <SettingsRow
                title="Timer"
                description="Uptime counts from when the stream went live."
                onReset={reset('timer')}
                control={
                  <SegmentedSelect
                    value={draft.timer}
                    onChange={(v) => change('timer', v)}
                    onPreview={preview('timer')}
                    options={[
                      { value: 'watching', label: 'Time watching' },
                      { value: 'uptime', label: 'Uptime' },
                      { value: 'off', label: 'Off' },
                    ]}
                  />
                }
              />
            </SettingsSection>

            <SettingsSection label="Buttons">
              <SettingsRow
                title="Watch button"
                description="Opens the channel, VOD or clip for friends who click it."
                onReset={reset('watch_button')}
                control={<Toggle enabled={draft.watch_button} onChange={() => change('watch_button', !draft.watch_button)} ariaLabel="Watch button" />}
              >
                {draft.watch_button && (
                  <SubControls>
                    <SubControl
                      title="Label"
                      stacked
                      onReset={reset('watch_button_label')}
                      control={
                        <TemplateInput
                          value={draft.watch_button_label}
                          onChange={(v) => change('watch_button_label', v)}
                          tokens={(model?.tokens ?? []).filter((t) => t.token === '{kind}' || t.token === '{streamer}' || t.token === '{platform}')}
                          ariaLabel="Watch button label"
                          max={32}
                        />
                      }
                    />
                  </SubControls>
                )}
              </SettingsRow>
              <SettingsRow
                title="StreamNook button"
                titleBadge={<SubscriberPill locked={!member} />}
                description={
                  member ? 'A link to streamnook.app with a rotating label.' : 'A link to streamnook.app. Subscribers can turn it off.'
                }
                onReset={member ? reset('promo_button') : undefined}
                control={
                  <Toggle
                    enabled={member ? draft.promo_button : true}
                    onChange={() => change('promo_button', !draft.promo_button)}
                    disabled={!member}
                    ariaLabel="StreamNook button"
                  />
                }
              />
            </SettingsSection>

            <SettingsSection label="Links">
              <SettingsRow
                title="Clickable text"
                description="The top line, and a streamer picture, open the channel."
                onReset={reset('link_text')}
                control={<Toggle enabled={draft.link_text} onChange={() => change('link_text', !draft.link_text)} ariaLabel="Clickable text" />}
              />
            </SettingsSection>
          </div>

          <div ref={rightRef} className="flex min-w-0 flex-1 flex-col items-center gap-4 overflow-y-auto border-l border-white/[0.06] bg-black/10 px-5 py-5">
            <div className="flex flex-wrap justify-center gap-1" role="tablist" aria-label="Preview">
              {TABS.map((t) => (
                <button
                  key={t.id}
                  type="button"
                  role="tab"
                  aria-selected={shownTab === t.id}
                  onClick={() => setTab(t.id)}
                  className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-[12px] font-medium transition-colors ${
                    shownTab === t.id ? 'bg-white/[0.1] text-textPrimary' : 'text-textSecondary hover:text-textPrimary'
                  }`}
                >
                  {model?.shown === t.id && enabled && (
                    <Tooltip content="Showing on Discord now">
                      <span className="h-1.5 w-1.5 rounded-full bg-[#23a55a]" />
                    </Tooltip>
                  )}
                  {t.label}
                </button>
              ))}
            </div>
            <div style={{ zoom: cardScale }}>
              <DiscordStatusPreview model={model} now={now} />
            </div>
            <div className="text-center">
              <div className="text-[12px] text-textSecondary">{status}</div>
              {model && !model.live && (
                <div className="mt-1 text-[11px] text-textMuted">A sample: Discord shows this tab when it matches what you watch.</div>
              )}
              <div className="mt-1 text-[11px] text-textMuted">Friends see the buttons; Discord hides them on your own profile.</div>
            </div>
          </div>
        </div>
      </motion.div>
    </motion.div>,
    document.body,
  );
}
