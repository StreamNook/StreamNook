// Overlay builder: the WYSIWYG design studio for the OBS chat overlay, the same
// component in the desktop app (Settings > Stream Overlay) and on
// streamnook.app/overlays. Left: controls. Right: a large scaled preview that
// renders the SAME renderer the hosted overlay uses (OverlayChat) at the chosen
// canvas size, so streamers see exactly how many chats fit and what viewers will
// see. Multi-source: add Twitch/Kick/YouTube/TikTok channels and preview the
// merged feed. Everything that differs between the app and the site comes from
// the OverlayHost (./overlayHost); this file must never import Tauri.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { RotateCcw, RefreshCw, Link2, Plus, X, AlertTriangle, Play, Pause, Copy, Trash2, Pencil, Check, ChevronRight, LogIn } from 'lucide-react';
import { Tooltip } from '../../ui/Tooltip';
import { Dropdown } from '../../ui/Dropdown';
import { SettingsSection, SettingsRow, SettingsSubGroup, SegmentedSelect } from '../../settings/_primitives';
import { SevenTVLogo } from '../../emotesets/SevenTVLogo';
import streamNookLogo from '../../../assets/streamnook-logo.png';
import { OverlayChat } from '../OverlayChat';
import { ProviderIcon } from '../ProviderIcon';
import { SAMPLE_MESSAGES, randomSampleMessage, seedFlowMessages, type OverlayMessage } from '../sampleMessages';
import {
  BUBBLE_SHAPES,
  CATEGORY_TEMPLATE_TOKENS,
  DEFAULT_LINK_COLOR,
  DEFAULT_OVERLAY_STYLE,
  EMOJI_STYLES,
  EVENT_CATEGORIES,
  EVENT_TEMPLATE_EXAMPLES,
  EVENT_TEMPLATE_TOKENS,
  sampleContextFor,
  renderEventTemplate,
  LINK_STYLES,
  REPLY_STYLES,
  FONT_OPTIONS,
  OVERLAY_ANIMATIONS,
  CHEER_DISPLAYS,
  GIANT_EMOTE_ALIGNS,
  OVERLAY_ENTRANCES,
  OVERLAY_LIMITS,
  OVERLAY_TEXT_ALIGNS,
  OVERLAY_TEXT_WEIGHTS,
  PROVIDER_CATEGORY_LABELS,
  PROVIDER_EVENT_CATEGORIES,
  THIRD_PARTY_BADGE_PROVIDERS,
  clampOverlayStyle,
  type EventCategory,
  type OverlayStyle,
} from '../overlayConfig';
import { CURRENCY_OPTIONS } from '../currency';
import { PROVIDERS, type ProviderId } from '../../../types/providers';
import {
  isTwitchLogin,
  isYouTubeChannelId,
  isYouTubeLegacyPath,
  kickSlugFromInput,
  linkPlatformOf,
  kickSlugHasTwoSpellings,
  parseTikTokIdentifier,
  parseTwitchLink,
  parseYouTubeIdentifier,
} from '../../../utils/parseChannelInput';
import { useOverlayHost } from './overlayHost';
import { newProfileUid, profileFromServer, reconcileProfiles, type OverlayProfile, type OverlaySource, type ServerOverlay } from './profileSync';

const STORAGE_KEY = 'sn_overlay_style_v1';
const SOURCES_KEY = 'sn_overlay_sources_v1';
// The published overlay's opaque id, remembered so re-publishing UPDATES the same
// row (the OBS link the streamer already pasted stays valid) instead of minting a
// new link each time.
const OVERLAY_ID_KEY = 'sn_overlay_id_v1';
// Overlay rows live behind the Twitch-authenticated streamnook.app API. The host
// makes the calls: on desktop Rust holds the token, on the site the session
// cookie proves the account.
const PUBLISH_PATH = '/api/overlays';
const SOURCE_PROVIDERS: ProviderId[] = ['twitch', 'kick', 'youtube', 'tiktok'];

function loadOverlayId(): string | null {
  try { return localStorage.getItem(OVERLAY_ID_KEY); } catch { return null; }
}


const PROVIDER_LABEL: Partial<Record<ProviderId, string>> = {
  twitch: 'Twitch', kick: 'Kick', youtube: 'YouTube', tiktok: 'TikTok',
};
const providerLabel = (p: ProviderId): string => PROVIDER_LABEL[p] ?? p;

// Marks a setting that only takes effect on certain platforms (avatars are a
// YouTube/TikTok thing, first-time chatter signals are Twitch-only, and so on).
// The source logos read at a glance and the tooltip spells it out, so a streamer
// isn't left wondering why a toggle did nothing for their platform.
const SourceScope = ({ sources }: { sources: ProviderId[] }) => (
  <Tooltip content={`Only affects ${sources.map(providerLabel).join(' & ')}`}>
    <span className="inline-flex items-center gap-1 opacity-60" aria-label={`Only affects ${sources.map(providerLabel).join(' and ')}`}>
      {sources.map((p) => <ProviderIcon key={p} provider={p} size="0.95em" />)}
    </span>
  </Tooltip>
);

const SIZE_PRESETS: { label: string; width: number; height: number }[] = [
  { label: 'Standard', width: 400, height: 640 },
  { label: 'Tall', width: 380, height: 1000 },
  { label: 'Wide', width: 620, height: 520 },
  { label: 'Full column', width: 380, height: 1440 },
  // A low strip for laying chat over a gameplay scene without eating its height.
  { label: 'Banner', width: 620, height: 160 },
];

// Whether a setting still holds its default. Compares arrays element-wise and
// objects key-wise (order-independent), since several settings are lists or maps
// whose identity changes on every edit even when the contents match.
const sameAsDefault = (a: unknown, b: unknown): boolean => {
  if (a === b) return true;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((v, i) => sameAsDefault(v, b[i]));
  }
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    const ka = Object.keys(a as object);
    const kb = Object.keys(b as object);
    if (ka.length !== kb.length) return false;
    return ka.every((k) => sameAsDefault((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
  }
  return false;
};

const SOURCE_PLACEHOLDER: Record<ProviderId, string> = {
  twitch: 'Twitch login (e.g. sodapoppin)',
  kick: 'Kick channel (e.g. trainwreckstv)',
  youtube: 'YouTube channel or link (e.g. mrbeast)',
  tiktok: 'TikTok @handle or LIVE link',
  rumble: 'Rumble channel',
  x: 'X handle',
};

const loadStyle = (): OverlayStyle => {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      // clampOverlayStyle also migrates legacy global event hides into the
      // per-source hiddenProviderEvents, so the state matches what renders.
      return clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, ...JSON.parse(raw) } as OverlayStyle);
    }
  } catch { /* ignore malformed */ }
  return { ...DEFAULT_OVERLAY_STYLE };
};

const loadSources = (): OverlaySource[] => {
  try {
    const raw = localStorage.getItem(SOURCES_KEY);
    if (raw) return JSON.parse(raw);
  } catch { /* ignore */ }
  return [];
};

// ── Multi-overlay profiles ──────────────────────────────────────────────────
// Each profile is its own published overlay: its own id (OBS link), style, and
// sources — so a streamer can run e.g. a clean face-cam overlay and a loud
// event-wall overlay side by side. The legacy single-overlay keys migrate into
// profile 0 on first load, and keep tracking the ACTIVE profile so anything
// still reading them sees the overlay currently being edited. The profile name
// travels inside the published style (`profileName`) so a fresh machine
// recovers names along with configs.
const PROFILES_KEY = 'sn_overlay_profiles_v1';
const ACTIVE_PROFILE_KEY = 'sn_overlay_active_v1';

function loadProfiles(): { profiles: OverlayProfile[]; active: number } {
  try {
    const raw = localStorage.getItem(PROFILES_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<OverlayProfile>[];
      if (Array.isArray(parsed) && parsed.length > 0) {
        const usedUids = new Set<string>();
        const profiles = parsed.map((p, i) => {
          const uid = typeof p?.uid === 'string' && p.uid && !usedUids.has(p.uid) ? p.uid : newProfileUid();
          usedUids.add(uid);
          return {
            uid,
            name: typeof p?.name === 'string' && p.name.trim() ? p.name : `Overlay ${i + 1}`,
            id: typeof p?.id === 'string' && p.id ? p.id : null,
            version: typeof p?.version === 'string' && p.version ? p.version : null,
            style: clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, ...(p?.style ?? {}) } as OverlayStyle),
            sources: Array.isArray(p?.sources) ? (p.sources as OverlaySource[]) : [],
          };
        });
        // Repair: no two profiles may claim the same published row (a pre-fix
        // race could stamp one row's id onto two profiles). First claim keeps
        // the link; later claimants go unpublished so their next publish mints
        // a fresh row.
        const seenIds = new Set<string>();
        for (const p of profiles) {
          if (!p.id) continue;
          if (seenIds.has(p.id)) { p.id = null; p.version = null; }
          else seenIds.add(p.id);
        }
        const stored = parseInt(localStorage.getItem(ACTIVE_PROFILE_KEY) || '0', 10);
        const active = Math.min(profiles.length - 1, Math.max(0, Number.isFinite(stored) ? stored : 0));
        return { profiles, active };
      }
    }
  } catch { /* fall through to migration */ }
  // First run on this build (or unreadable list): adopt the legacy keys.
  return {
    profiles: [{ uid: newProfileUid(), name: 'Default', id: loadOverlayId(), version: null, style: loadStyle(), sources: loadSources() }],
    active: 0,
  };
}

const Toggle = ({ enabled, onChange }: { enabled: boolean; onChange: () => void }) => (
  <button
    onClick={onChange}
    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors flex-shrink-0 ${enabled ? 'bg-accent' : 'bg-gray-600'}`}
  >
    <span className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${enabled ? 'translate-x-6' : 'translate-x-1'}`} />
  </button>
);

const Slider = ({
  value, min, max, step = 1, onChange, format,
}: {
  value: number; min: number; max: number; step?: number;
  onChange: (v: number) => void; format?: (v: number) => string;
}) => (
  <div className="flex items-center gap-3 w-full">
    <input
      type="range" min={min} max={max} step={step} value={value}
      onChange={(e) => onChange(parseFloat(e.target.value))}
      className="flex-1 accent-accent cursor-pointer"
    />
    <span className="w-16 text-right text-[12px] tabular-nums text-textSecondary">
      {format ? format(value) : value}
    </span>
  </div>
);

type SceneBg = 'scene' | 'checker' | 'dark' | 'light';

const SCENE_STYLES: Record<SceneBg, CSSProperties> = {
  // A soft "studio" backdrop so the overlay reads as sitting in a real scene,
  // not floating in empty space. The faint grid is drawn by an overlaid element.
  scene: {
    background:
      'radial-gradient(120% 90% at 72% 12%, rgba(84,74,150,0.28), transparent 55%), radial-gradient(90% 80% at 12% 92%, rgba(29,158,117,0.18), transparent 55%), linear-gradient(160deg, #171a20, #0c0e12)',
  },
  checker: {
    backgroundColor: '#2a2a30',
    backgroundImage:
      'linear-gradient(45deg, #3a3a42 25%, transparent 25%), linear-gradient(-45deg, #3a3a42 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #3a3a42 75%), linear-gradient(-45deg, transparent 75%, #3a3a42 75%)',
    backgroundSize: '20px 20px',
    backgroundPosition: '0 0, 0 10px, 10px -10px, -10px 0',
  },
  dark: { background: 'linear-gradient(135deg, #12121a, #1c1030)' },
  light: { background: 'linear-gradient(135deg, #dfe4ee, #c3ccdd)' },
};

// Appends a random chatter's message on a jittered timer so the preview reads
// like a live chat. OverlayChat caps to what fits and animates each new row, so
// this just grows the list (bounded) and lets the renderer do the rest.
const SampleFlowFeed = ({ style }: { style: OverlayStyle }) => {
  const [msgs, setMsgs] = useState<OverlayMessage[]>(() => seedFlowMessages(8));
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const tick = () => {
      setMsgs((prev) => [...prev, randomSampleMessage()].slice(-60));
      // Jittered cadence so it feels organic, not metronomic.
      timer = setTimeout(tick, 850 + Math.random() * 1700);
    };
    timer = setTimeout(tick, 600);
    return () => clearTimeout(timer);
  }, []);
  return <OverlayChat messages={msgs} style={style} superSample={2} />;
};

// A YouTube source kept as a UC id (a /channel/ or legacy /c/ link, or a typed id)
// reads as a string of letters, so the row shows the channel's name once YouTube
// gives it. Display only: the stored and published source stays the id.
function useYouTubeSourceTitle(source: OverlaySource): string | null {
  const id = source.provider === 'youtube' && isYouTubeChannelId(source.channel) ? source.channel : null;
  const host = useOverlayHost();
  const [named, setNamed] = useState<{ id: string; title: string } | null>(null);
  useEffect(() => {
    if (!id) return;
    let current = true;
    void host.youTubeChannelTitle(id).then((title) => {
      if (current && title) setNamed({ id, title });
    });
    return () => {
      current = false;
    };
  }, [id, host]);
  return named && named.id === id ? named.title : null;
}

// A plain source row: platform + channel + remove. Blocking lives in the Filters
// tab now (BlockRow), so this stays a clean list of where chat comes from.
const SourceRow = ({ source, onRemove }: { source: OverlaySource; onRemove: () => void }) => {
  const title = useYouTubeSourceTitle(source);
  return (
    <div className="flex items-center gap-2 rounded-lg bg-glass px-2.5 py-1.5">
      <ProviderIcon provider={source.provider} size="14px" />
      <span className="text-sm text-textPrimary truncate flex-1">{title ?? source.channel}</span>
      <button onClick={onRemove} className="text-textSecondary hover:text-textPrimary flex-shrink-0">
        <X size={14} />
      </button>
    </div>
  );
};

// The full token reference. Collapsed by default — it's a lookup table, not
// something to read every visit. Each row leads with the value the token stands
// in for, because a name plus a description still leaves you guessing what you'd
// actually get; the example answers that outright.
const TokenLegend = () => {
  const [open, setOpen] = useState(false);
  const groups = EVENT_TEMPLATE_TOKENS.reduce<Record<string, typeof EVENT_TEMPLATE_TOKENS>>((acc, t) => {
    (acc[t.group] ??= []).push(t);
    return acc;
  }, {});
  return (
    <div>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="inline-flex items-center gap-1 text-[12px] text-textSecondary hover:text-textPrimary transition-colors"
      >
        <ChevronRight size={12} className={`transition-transform ${open ? 'rotate-90' : ''}`} />
        {open ? 'Hide' : 'Show'} every token and what it becomes
      </button>
      {open && (
        <div className="mt-2.5 space-y-3">
          {Object.entries(groups).map(([group, items]) => (
            <div key={group}>
              <div className="text-[11px] font-semibold uppercase tracking-[0.12em] text-textMuted mb-1">
                {group}
              </div>
              <div className="space-y-1">
                {items.map((t) => (
                  <div key={t.token} className="text-[12px] leading-snug">
                    <div className="flex items-baseline gap-1.5 flex-wrap">
                      <code className="font-mono text-[11.5px] text-textPrimary">{`{${t.token}}`}</code>
                      <span className="text-textMuted">becomes</span>
                      <span className="text-textPrimary font-medium">{t.example}</span>
                    </div>
                    <div className="text-textSecondary text-[11.5px]">{t.label}</div>
                  </div>
                ))}
              </div>
            </div>
          ))}
          <p className="text-[11.5px] leading-relaxed text-textMuted">
            Not every event carries every value. If an event is missing something your
            text asks for, that one event keeps the platform's own message instead, so
            nothing ever goes out with a gap where a number should be.
          </p>
        </div>
      )}
    </div>
  );
};

// Custom wording for one event category. Tokens are chips rather than something
// to memorize: clicking one drops it at the cursor, and the line underneath shows
// the sentence filled in with sample values so you can see what you're writing
// before an event ever fires.
const EventTemplateEditor = ({
  category,
  value,
  onChange,
}: {
  category: EventCategory;
  value: string;
  onChange: (next: string) => void;
}) => {
  const inputRef = useRef<HTMLInputElement>(null);
  const tokens = CATEGORY_TEMPLATE_TOKENS[category];

  // Insert at the caret, not the end — a token usually belongs mid-sentence, and
  // appending would make every chip click a retype.
  const insert = (token: string) => {
    const el = inputRef.current;
    const chunk = `{${token}}`;
    if (!el) { onChange(`${value}${chunk}`); return; }
    const start = el.selectionStart ?? value.length;
    const end = el.selectionEnd ?? start;
    const next = `${value.slice(0, start)}${chunk}${value.slice(end)}`;
    onChange(next);
    requestAnimationFrame(() => {
      el.focus();
      const caret = start + chunk.length;
      el.setSelectionRange(caret, caret);
    });
  };

  // What this text turns into, using the sample values from the legend. Typos are
  // the thing worth catching here: a token that doesn't exist would silently make
  // every real event fall back, so name it rather than just showing nothing.
  const preview = (() => {
    const text = value.trim();
    if (!text) return null;
    const unknown = [...text.matchAll(/\{([a-zA-Z]+)\}/g)]
      .map((m) => m[1])
      .filter((name) => !EVENT_TEMPLATE_TOKENS.some((t) => t.token === name));
    if (unknown.length) {
      return { error: `No such token: ${[...new Set(unknown)].map((u) => `{${u}}`).join(', ')}` };
    }
    const offered = new Set<string>(tokens);
    const foreign = [...text.matchAll(/\{([a-zA-Z]+)\}/g)]
      .map((m) => m[1])
      .filter((name) => !offered.has(name));
    return {
      text: renderEventTemplate(text, sampleContextFor(category)) ?? text,
      // A real token that this event type never carries: valid syntax, but it
      // would make every one of these events fall back to the platform message.
      warn: foreign.length
        ? `${[...new Set(foreign)].map((f) => `{${f}}`).join(', ')} isn't part of this event, so it would always fall back`
        : null,
    };
  })();

  return (
    <div className="space-y-2">
      <input
        ref={inputRef}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        maxLength={200}
        placeholder={EVENT_TEMPLATE_EXAMPLES[category]}
        className="w-full glass-input rounded-md px-2.5 py-1.5 text-[13px] text-textPrimary placeholder:text-textMuted"
      />
      {preview && (
        <div className="text-[12px] leading-snug">
          {preview.error ? (
            <span className="text-red-400">{preview.error}</span>
          ) : (
            <>
              <span className="text-textMuted">Shows as </span>
              <span className="text-textPrimary">{preview.text}</span>
              {preview.warn && <div className="text-amber-400 mt-0.5">{preview.warn}</div>}
            </>
          )}
        </div>
      )}
      <div className="flex flex-wrap gap-1.5">
        {tokens.map((t) => {
          const meta = EVENT_TEMPLATE_TOKENS.find((x) => x.token === t);
          return (
            <Tooltip key={t} content={meta ? `${meta.label}, becomes "${meta.example}"` : String(t)}>
              <button
                type="button"
                onClick={() => insert(t)}
                style={{ borderRadius: 6 }}
                className="glass-button px-1.5 py-0.5 text-[11px] font-medium text-textSecondary hover:text-textPrimary transition-colors"
              >
                {`{${t}}`}
              </button>
            </Tooltip>
          );
        })}
      </div>
    </div>
  );
};

// A per-source hidden-accounts editor (Filters tab). Renders as a flat SettingsRow
// (channel as the row title, input + chips below) so it sits inline in the section
// card instead of a nested box-in-box.
const BlockRow = ({ source, blocked, onAddBlocked, onRemoveBlocked }: {
  source: OverlaySource;
  blocked: string[];
  onAddBlocked: (name: string) => void;
  onRemoveBlocked: (name: string) => void;
}) => {
  const [val, setVal] = useState('');
  const add = () => { const n = val.trim(); if (n) { onAddBlocked(n); setVal(''); } };
  return (
    <SettingsRow
      title={(
        <span className="inline-flex items-center gap-1.5">
          <ProviderIcon provider={source.provider} size="14px" /> {source.channel}
        </span>
      ) as unknown as string}
    >
      <div className="space-y-2">
        <label className="block text-[11px] text-textSecondary">Username or display name to hide</label>
        <div className="flex items-center gap-2">
          <input
            value={val}
            onChange={(e) => setVal(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); add(); } }}
            placeholder="Type a name"
            className="flex-1 min-w-0 rounded-lg bg-glass border border-borderLight px-3 py-1.5 text-sm text-textPrimary placeholder:text-textMuted focus:outline-none focus:border-accent/60"
          />
          <button onClick={add} className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium glass-input text-textPrimary flex-shrink-0">
            <Plus size={14} /> Hide
          </button>
        </div>
        {blocked.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {blocked.map((u) => (
              <span key={u} className="inline-flex items-center gap-1.5 rounded-lg bg-glass px-3 py-1 text-[13px] text-textSecondary">
                {u}
                <button onClick={() => onRemoveBlocked(u)} className="hover:text-textPrimary"><X size={14} /></button>
              </span>
            ))}
          </div>
        )}
      </div>
    </SettingsRow>
  );
};

type CommandMode = 'prefix' | 'exact';
type CommandFilter = { value: string; mode: CommandMode };

// The command-filter list editor (Filters tab): pick Prefix (hide every command
// starting with a character) or Exact (hide one specific command), type it, and
// it's added as a labeled, removable chip. No guessing — you choose the mode.
const CommandFilterEditor = ({ filters, onAdd, onRemove }: {
  filters: CommandFilter[];
  onAdd: (value: string, mode: CommandMode) => void;
  onRemove: (value: string, mode: CommandMode) => void;
}) => {
  const [val, setVal] = useState('');
  const [mode, setMode] = useState<CommandMode>('prefix');
  const add = () => { const t = val.trim(); if (t) { onAdd(t, mode); setVal(''); } };
  return (
    <div className="w-full space-y-2">
      <SegmentedSelect
        value={mode}
        onChange={setMode}
        options={[{ value: 'prefix', label: 'Prefix' }, { value: 'exact', label: 'Exact command' }]}
      />
      <label className="block text-[11px] text-textSecondary">{mode === 'prefix' ? 'Prefix character' : 'Command to hide'}</label>
      <div className="flex items-center gap-2">
        <input
          value={val}
          onChange={(e) => setVal(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); add(); } }}
          placeholder={mode === 'prefix' ? '! or #' : '!title'}
          className="flex-1 min-w-0 rounded-lg bg-glass border border-borderLight px-3 py-1.5 text-sm text-textPrimary placeholder:text-textMuted focus:outline-none focus:border-accent/60"
        />
        <button onClick={add} className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium glass-input text-textPrimary flex-shrink-0">
          <Plus size={14} /> Add
        </button>
      </div>
      {filters.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {filters.filter((f) => f?.value).map((f, i) => (
            <span key={`${f.mode}:${f.value}:${i}`} className="inline-flex items-center gap-1.5 rounded-lg bg-glass px-3 py-1 text-[13px]">
              <span className="font-medium text-textPrimary">{f.value}</span>
              <span className="text-textMuted">{f.mode === 'prefix' ? 'all commands' : 'exact'}</span>
              <button onClick={() => onRemove(f.value, f.mode)} className="text-textSecondary hover:text-textPrimary"><X size={14} /></button>
            </span>
          ))}
        </div>
      )}
      <p className="text-[12px] leading-relaxed text-textMuted">
        <span className="text-textSecondary">Prefix</span> hides every command starting with that character. <span className="text-textSecondary">Exact command</span> hides only that one.
      </p>
    </div>
  );
};

// Word/phrase blocklist editor (Filters tab): type a phrase, it's added as a
// removable chip. Matching is case-insensitive substring, done in the renderer.
const PhraseEditor = ({ phrases, onAdd, onRemove }: {
  phrases: string[];
  onAdd: (value: string) => void;
  onRemove: (value: string) => void;
}) => {
  const [val, setVal] = useState('');
  const add = () => { const t = val.trim(); if (t) { onAdd(t); setVal(''); } };
  return (
    <div className="w-full space-y-2">
      <div className="flex items-center gap-2">
        <input
          value={val}
          onChange={(e) => setVal(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); add(); } }}
          placeholder="Word or phrase to hide"
          className="flex-1 min-w-0 rounded-lg bg-glass border border-borderLight px-3 py-1.5 text-sm text-textPrimary placeholder:text-textMuted focus:outline-none focus:border-accent/60"
        />
        <button onClick={add} className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium glass-input text-textPrimary flex-shrink-0">
          <Plus size={14} /> Add
        </button>
      </div>
      {phrases.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {phrases.map((p) => (
            <span key={p} className="inline-flex items-center gap-1.5 rounded-lg bg-glass px-3 py-1 text-[13px] text-textSecondary">
              {p}
              <button onClick={() => onRemove(p)} className="hover:text-textPrimary"><X size={14} /></button>
            </span>
          ))}
        </div>
      )}
    </div>
  );
};

const sourceKey = (s: OverlaySource) => `${s.provider}:${s.channel.toLowerCase()}`;

// Sentinel dropdown value + starter for the custom-font option, and a helper to
// pull the bare family name out of a font-family string for the text input.
const CUSTOM_FONT = '__custom__';
const CUSTOM_FONT_STARTER = "'Poppins', sans-serif";
const primaryFamilyName = (ff: string) => (ff || '').split(',')[0].trim().replace(/^["']|["']$/g, '');

// A few sample emoji shown in the Emoji-style dropdown so users can compare vendor
// styles at a glance. Built from codepoints (no literal emoji in source).
const EMOJI_SAMPLES = [0x1f600, 0x1f602, 0x1f60d].map((cp) => ({
  cp: cp.toString(16),
  char: String.fromCodePoint(cp),
}));

const OverlayEditor = () => {
  const host = useOverlayHost();
  const apiRequest = host.request;
  // One load, shared by every initializer below (useState initials only read on
  // the first render, so the snapshot never goes stale).
  const initial = useMemo(loadProfiles, []);
  const [profiles, setProfiles] = useState<OverlayProfile[]>(initial.profiles);
  const [activeIdx, setActiveIdx] = useState(initial.active);
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState('');
  const [confirmDelete, setConfirmDelete] = useState(false);
  // No pinned/unpinned state any more. The switcher is a CARD now, so it is
  // the same object whether it is resting in the page or holding the top of
  // the scroller, and there is nothing to fade in. The sentinel and its
  // IntersectionObserver existed only to drive the old veil's opacity.
  const [style, setStyle] = useState<OverlayStyle>(initial.profiles[initial.active].style);
  // Reset-everything is armed by a first click and disarms on its own.
  const [resetArmed, setResetArmed] = useState(false);
  useEffect(() => {
    if (!resetArmed) return;
    const t = setTimeout(() => setResetArmed(false), 4000);
    return () => clearTimeout(t);
  }, [resetArmed]);
  const [flow, setFlow] = useState(false);
  const [sources, setSources] = useState<OverlaySource[]>(initial.profiles[initial.active].sources);
  const [sceneBg, setSceneBg] = useState<SceneBg>('scene');
  const [previewMode, setPreviewMode] = useState<'sample' | 'live'>('sample');
  const [addProvider, setAddProvider] = useState<ProviderId>('twitch');
  const [addChannel, setAddChannel] = useState('');
  const [addError, setAddError] = useState<string | null>(null);
  // Set while an input is looked up (a legacy YouTube link, or a Kick name with
  // two spellings), so Add can't double-fire.
  const [addBusy, setAddBusy] = useState(false);
  const [publishState, setPublishState] = useState<'idle' | 'publishing' | 'done' | 'error'>('idle');
  const [publishError, setPublishError] = useState<string | null>(null);
  const [publishedUrl, setPublishedUrl] = useState<string | null>(
    initial.profiles[initial.active].id ? `https://streamnook.app/overlay/${initial.profiles[initial.active].id}` : null,
  );
  // The ACTIVE profile's published id; re-publish updates the same link.
  const overlayIdRef = useRef<string | null>(initial.profiles[initial.active].id);
  // Which profile the editor is showing, by stable uid. Publish requests never
  // read this (they build from their render closure); a response consults it to
  // decide whether the editor is still on the profile the push was for.
  const activeUidRef = useRef<string>(initial.profiles[initial.active].uid);

  // The scaled stage measures its own width so the overlay canvas fits the pane
  // at true proportions (scaled down when the canvas is wider than the pane).
  const stageWrapRef = useRef<HTMLDivElement>(null);
  const [stageW, setStageW] = useState(360);
  useLayoutEffect(() => {
    const el = stageWrapRef.current;
    if (!el) return;
    // Measure synchronously BEFORE paint so the first frame already uses the right
    // scale. Otherwise it paints at the default guess, then the observer corrects
    // the width and the whole canvas visibly jumps — and re-scaling a painted frame
    // leaves the text blurry until the next full repaint.
    setStageW(el.clientWidth);
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver((entries) => {
      setStageW(entries[entries.length - 1].contentRect.width);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  // Cap the stage to the available viewport height so a tall overlay scales down
  // to fit instead of clipping into the settings window.
  const [viewportH, setViewportH] = useState(() => (typeof window !== 'undefined' ? window.innerHeight : 900));
  useEffect(() => {
    const onResize = () => setViewportH(window.innerHeight);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);
  const maxStageH = Math.max(340, viewportH - 240);
  // Leave an inset around the canvas so it sits framed inside the scene, not edge to edge.
  const STAGE_PAD = 44;
  const scale = Math.min(1, (stageW - STAGE_PAD) / style.width, (maxStageH - STAGE_PAD) / style.height);

  // The working state (style/sources/id) IS the active profile: mirror every
  // change into the profiles list, and keep writing the legacy single-overlay
  // keys so anything still reading them sees the overlay being edited.
  useEffect(() => {
    setProfiles((list) =>
      list.map((p, i) => (i === activeIdx ? { ...p, style, sources } : p)),
    );
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(style));
      localStorage.setItem(SOURCES_KEY, JSON.stringify(sources));
      if (overlayIdRef.current) localStorage.setItem(OVERLAY_ID_KEY, overlayIdRef.current);
    } catch { /* ignore */ }
  }, [style, sources, activeIdx]);
  useEffect(() => {
    try {
      localStorage.setItem(PROFILES_KEY, JSON.stringify(profiles));
      localStorage.setItem(ACTIVE_PROFILE_KEY, String(activeIdx));
    } catch { /* ignore */ }
  }, [profiles, activeIdx]);

  const set = <K extends keyof OverlayStyle>(key: K, val: OverlayStyle[K]) =>
    setStyle((s) => ({ ...s, [key]: val }));

  // Per-row reset. Returns undefined while every key it covers still matches the
  // StreamNook default, so SettingsRow shows nothing — the icon appearing IS the
  // "you changed this" signal. Takes every key a row controls so a row with more
  // than one control resets as a unit.
  const resetFor = (...keys: (keyof OverlayStyle)[]) => {
    if (keys.every((k) => sameAsDefault(style[k], DEFAULT_OVERLAY_STYLE[k]))) return undefined;
    return () =>
      setStyle((s) => {
        const next = { ...s };
        for (const k of keys) (next[k] as OverlayStyle[typeof k]) = DEFAULT_OVERLAY_STYLE[k];
        return next;
      });
  };

  // Per-source event hide, keyed `provider:category`.
  const toggleProviderEvent = (key: string) =>
    setStyle((s) => {
      const hidden = s.hiddenProviderEvents ?? [];
      return { ...s, hiddenProviderEvents: hidden.includes(key) ? hidden.filter((c) => c !== key) : [...hidden, key] };
    });

  const toggleBadgeProvider = (id: string) =>
    setStyle((s) => {
      const hidden = s.hiddenBadgeProviders ?? [];
      return { ...s, hiddenBadgeProviders: hidden.includes(id) ? hidden.filter((k) => k !== id) : [...hidden, id] };
    });

  const toggleSourceFilter = (id: ProviderId) =>
    setStyle((s) => {
      const has = s.sources.includes(id);
      const next = has ? s.sources.filter((p) => p !== id) : [...s.sources, id];
      return { ...s, sources: next.length ? next : s.sources };
    });

  const addSource = async () => {
    const raw = addChannel.trim();
    if (!raw || addBusy) return;
    // A pasted link names its own platform, whatever the dropdown shows, so a
    // kick.com link typed with Twitch selected still adds the Kick channel. The
    // dropdown follows so the row that appears matches what it says.
    const linked = linkPlatformOf(raw);
    const provider = linked ?? addProvider;
    if (linked && linked !== addProvider) setAddProvider(linked);
    // Resolve the input to what each provider actually connects by. The dropdown
    // has already named the platform, so a typed handle is safe to accept here
    // (MultiChat's add box takes links only, because its bare words are Twitch).
    let channel: string | null;
    if (provider === 'youtube') {
      channel = parseYouTubeIdentifier(raw);
      if (!channel) { setAddError('Enter a YouTube @handle, channel link, or live video link.'); return; }
      if (isYouTubeLegacyPath(channel)) {
        // A /c/ or /user/ link names no channel until YouTube says which.
        setAddBusy(true);
        try {
          channel = await host.resolveYouTubeIdentifier(channel);
        } catch (err) {
          setAddError(host.lookupError(err, 'YouTube'));
          return;
        } finally {
          setAddBusy(false);
        }
      }
    } else if (provider === 'tiktok') {
      channel = parseTikTokIdentifier(raw);
      if (!channel) { setAddError('Enter a TikTok @handle or LIVE link.'); return; }
    } else if (provider === 'kick') {
      // Cleaned the way MultiChat cleans it, then looked up when a typed
      // "some_name" could be some-name on Kick.
      channel = kickSlugFromInput(raw);
      if (!channel) { setAddError('Enter a Kick channel name or link.'); return; }
      if (kickSlugHasTwoSpellings(channel)) {
        setAddBusy(true);
        try {
          channel = await host.resolveKickSlug(channel);
        } catch (err) {
          setAddError(host.lookupError(err, 'Kick'));
          return;
        } finally {
          setAddBusy(false);
        }
      }
    } else {
      // A Twitch login, typed or read off a pasted channel link.
      channel = parseTwitchLink(raw) ?? raw.replace(/^#/, '').toLowerCase();
      if (!isTwitchLogin(channel)) { setAddError('Enter a Twitch channel name or link.'); return; }
    }
    setAddError(null);
    const chan = channel;
    setSources((list) =>
      list.some((s) => s.provider === provider && s.channel.toLowerCase() === chan.toLowerCase())
        ? list
        : [...list, { provider, channel: chan }],
    );
    setAddChannel('');
  };

  const removeSource = (src: OverlaySource) => {
    setSources((list) => list.filter((s) => !(s.provider === src.provider && s.channel === src.channel)));
    // Drop that source's blocklist so removed sources don't leave orphan entries.
    setStyle((st) => {
      const key = sourceKey(src);
      if (!st.blockedUsers?.[key]) return st;
      const next = { ...st.blockedUsers };
      delete next[key];
      return { ...st, blockedUsers: next };
    });
  };

  const addBlockedUser = (src: OverlaySource, name: string) =>
    setStyle((st) => {
      const key = sourceKey(src);
      const cur = st.blockedUsers?.[key] ?? [];
      const n = name.trim().replace(/^@+/, '');
      if (!n || cur.some((x) => x.toLowerCase() === n.toLowerCase())) return st;
      return { ...st, blockedUsers: { ...st.blockedUsers, [key]: [...cur, n] } };
    });

  const removeBlockedUser = (src: OverlaySource, name: string) =>
    setStyle((st) => {
      const key = sourceKey(src);
      const cur = st.blockedUsers?.[key] ?? [];
      return { ...st, blockedUsers: { ...st.blockedUsers, [key]: cur.filter((x) => x !== name) } };
    });

  const addPhrase = (value: string) =>
    setStyle((s) => {
      const cur = s.hidePhrases ?? [];
      const v = value.trim();
      if (!v || cur.some((x) => x.toLowerCase() === v.toLowerCase())) return s;
      return { ...s, hidePhrases: [...cur, v] };
    });
  const removePhrase = (value: string) =>
    setStyle((s) => ({ ...s, hidePhrases: (s.hidePhrases ?? []).filter((x) => x !== value) }));

  const addCommandFilter = (value: string, mode: 'prefix' | 'exact') =>
    setStyle((s) => {
      const cur = s.commandFilters ?? [];
      const v = value.trim();
      if (!v || cur.some((x) => x.mode === mode && x.value.toLowerCase() === v.toLowerCase())) return s;
      return { ...s, commandFilters: [...cur, { value: v, mode }] };
    });
  const removeCommandFilter = (value: string, mode: 'prefix' | 'exact') =>
    setStyle((s) => ({ ...s, commandFilters: (s.commandFilters ?? []).filter((x) => !(x.value === value && x.mode === mode)) }));

  // ── Saving ────────────────────────────────────────────────────────────────
  // A published overlay belongs to the Twitch account and may be open in the app
  // and on the site at once. Each save carries the version it last saw; the
  // server refuses one made on top of a newer version (409) or to an overlay
  // deleted elsewhere (410), and the builder shows the account's copy instead.
  //
  // The version is read at send time from `versionsRef`, never from a render
  // closure, and only one save per overlay is in flight: an edit that lands
  // meanwhile is saved right after with the version the first save produced.
  // Two overlapping saves would otherwise carry the same version and the second
  // would be refused as a conflict with our own first.
  const versionsRef = useRef(new Map<string, string | null>(initial.profiles.map((p) => [p.uid, p.version])));
  // The id each overlay was published under, set the moment a save answers, so a
  // save queued behind a first publish updates that overlay instead of minting
  // a second one (state and profilesRef only catch up on the next render).
  const idsRef = useRef(new Map<string, string>());
  const inFlightRef = useRef(new Set<string>());
  const againRef = useRef(new Map<string, OverlayProfile>());
  // The debounced auto-save, holding exactly what it will send, so switching
  // overlays or closing the builder can send it now instead of dropping it.
  const pendingSaveRef = useRef<{ timer: ReturnType<typeof setTimeout>; profile: OverlayProfile } | null>(null);
  // Loading an overlay (on open, on switch, or a newer copy from the account)
  // changes the editor without the user editing anything; that must not count
  // as an edit and save straight back. True at mount for the same reason.
  const skipSaveRef = useRef(true);
  const profilesRef = useRef(profiles);
  useEffect(() => {
    profilesRef.current = profiles;
  }, [profiles]);
  // A line under the publish controls when the account's copy replaced this one.
  const [notice, setNotice] = useState<string | null>(null);

  const saveError = (err?: string, status?: number): string =>
    err === 'unauthenticated' ? 'Sign in to Twitch to publish an overlay.'
      : err === 'no_channels' ? 'Add at least one source first.'
        : err === 'overlay_limit' ? 'Overlay limit reached (10 per account). Delete one you no longer use first.'
          : `Publish failed (${err || status}).`;

  // Save one overlay. `copy` = the manual publish action (copies the OBS link and
  // shows state); auto-save passes false to update the same link silently, so the
  // published overlay always mirrors the builder.
  const pushProfile = async (forProfile: OverlayProfile, copy: boolean) => {
    const forUid = forProfile.uid;
    if (forProfile.sources.length === 0) {
      if (copy) { setPublishError('Add at least one source first.'); setPublishState('error'); }
      return;
    }
    if (inFlightRef.current.has(forUid)) {
      againRef.current.set(forUid, forProfile);
      return;
    }
    inFlightRef.current.add(forUid);
    if (copy) { setPublishState('publishing'); setPublishError(null); setNotice(null); }
    try {
      // Always `create`: an unusable id mints a fresh overlay rather than folding
      // this one into another overlay on the account. The name rides inside the
      // style so the other surfaces show it.
      const res = await apiRequest('POST', PUBLISH_PATH, undefined, {
        id: forProfile.id ?? undefined,
        create: true,
        base_updated_at: forProfile.id ? versionsRef.current.get(forUid) ?? undefined : undefined,
        channels: forProfile.sources,
        style: { ...forProfile.style, profileName: forProfile.name },
      });
      if (res.status === 409) {
        const body = res.json<{ error?: string; overlay?: ServerOverlay }>();
        if (body?.error === 'conflict' && body.overlay) {
          // Saved elsewhere since this copy was loaded: show the account's copy.
          const theirs = profileFromServer(body.overlay, forProfile.name, forUid);
          if (theirs) {
            againRef.current.delete(forUid);
            versionsRef.current.set(forUid, theirs.version);
            setProfiles((list) => list.map((p) => (p.uid === forUid ? theirs : p)));
            if (activeUidRef.current === forUid) {
              applyProfile(theirs);
              setNotice('This overlay was changed on another device, so it now shows that version.');
            }
            return;
          }
        }
        throw new Error(saveError(body?.error, res.status));
      }
      if (res.status === 410) {
        // Deleted elsewhere. It stays deleted; take it out of this list.
        againRef.current.delete(forUid);
        dropProfile(forUid, `"${forProfile.name}" was deleted on another device.`);
        return;
      }
      if (!res.ok) throw new Error(saveError(res.json<{ error?: string }>()?.error, res.status));
      const data = res.json<{ id: string; url: string; updated_at?: string }>();
      if (!data?.id || !data?.url) throw new Error('Publish failed (bad reply).');
      versionsRef.current.set(forUid, data.updated_at ?? null);
      idsRef.current.set(forUid, data.id);
      // Stamp the id and version onto the overlay this save was for, and strip
      // the id from any other entry that claims the same overlay.
      setProfiles((list) =>
        list.map((p) =>
          p.uid === forUid
            ? { ...p, id: data.id, version: data.updated_at ?? null }
            : p.id === data.id ? { ...p, id: null, version: null } : p,
        ),
      );
      if (activeUidRef.current === forUid) {
        overlayIdRef.current = data.id;
        try { localStorage.setItem(OVERLAY_ID_KEY, data.id); } catch { /* ignore */ }
        setPublishedUrl(data.url);
        if (copy) {
          try { await navigator.clipboard.writeText(data.url); } catch { /* clipboard may be blocked; URL still shown */ }
          setPublishState('done');
        }
      } else if (copy) {
        // Finished after the user switched away; don't paint result state onto
        // the overlay now on screen.
        setPublishState('idle');
      }
    } catch (e) {
      if (copy && activeUidRef.current === forUid) {
        setPublishError(e instanceof Error ? e.message : 'Publish failed.');
        setPublishState('error');
      }
    } finally {
      inFlightRef.current.delete(forUid);
      const next = againRef.current.get(forUid);
      if (next) {
        againRef.current.delete(forUid);
        void pushProfileRef.current({ ...next, id: next.id ?? idsRef.current.get(forUid) ?? null }, false);
      }
    }
  };
  // Timers and the flush-on-close path call the latest pushProfile.
  const pushProfileRef = useRef(pushProfile);
  useEffect(() => {
    pushProfileRef.current = pushProfile;
  });

  const flushPendingSave = () => {
    const pending = pendingSaveRef.current;
    if (!pending) return;
    clearTimeout(pending.timer);
    pendingSaveRef.current = null;
    void pushProfileRef.current(pending.profile, false);
  };
  const cancelPendingSave = (uid: string) => {
    if (pendingSaveRef.current?.profile.uid !== uid) return;
    clearTimeout(pendingSaveRef.current.timer);
    pendingSaveRef.current = null;
  };

  const publish = () => {
    const p = profiles[activeIdx];
    if (!p) return;
    cancelPendingSave(p.uid);
    void pushProfile({ ...p, style, sources }, true);
  };

  // Keep the published link a LIVE MIRROR of the builder: once published, save
  // any style/source change (debounced), so the streamer never has to re-copy.
  useEffect(() => {
    if (skipSaveRef.current) {
      skipSaveRef.current = false;
      return;
    }
    const p = profiles[activeIdx];
    if (!p?.id) return;
    if (pendingSaveRef.current) clearTimeout(pendingSaveRef.current.timer);
    const profile = { ...p, style, sources };
    const timer = setTimeout(() => {
      pendingSaveRef.current = null;
      void pushProfileRef.current(profile, false);
    }, 1500);
    pendingSaveRef.current = { timer, profile };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [style, sources]);
  // Closing the builder sends an edit still waiting on its debounce.
  useEffect(() => () => flushPendingSave(), []);

  // Load an overlay's saved state into the working editor state.
  const applyProfile = (p: OverlayProfile) => {
    skipSaveRef.current = true;
    activeUidRef.current = p.uid;
    overlayIdRef.current = p.id;
    setStyle(clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, ...p.style } as OverlayStyle));
    setSources(p.sources);
    setPublishedUrl(p.id ? `https://streamnook.app/overlay/${p.id}` : null);
    setPublishState('idle');
    setPublishError(null);
    setRenaming(false);
    setConfirmDelete(false);
  };

  // Take an overlay out of the list (deleted on another device), keeping the
  // editor on a neighbour and the list never empty.
  const dropProfile = (uid: string, message: string) => {
    cancelPendingSave(uid);
    const list = profilesRef.current;
    const idx = list.findIndex((p) => p.uid === uid);
    if (idx < 0) return;
    let next = list.filter((p) => p.uid !== uid);
    if (next.length === 0) {
      next = [{ uid: newProfileUid(), name: 'Default', id: null, version: null, style: { ...DEFAULT_OVERLAY_STYLE }, sources: [] }];
    }
    const wasActive = activeUidRef.current === uid;
    const keepUid = wasActive ? next[Math.max(0, Math.min(idx, next.length) - 1)].uid : activeUidRef.current;
    const nextIdx = Math.max(0, next.findIndex((p) => p.uid === keepUid));
    setProfiles(next);
    setActiveIdx(nextIdx);
    if (wasActive) applyProfile(next[nextIdx]);
    setNotice(message);
  };

  const switchProfile = (idx: number) => {
    if (idx === activeIdx || !profiles[idx]) return;
    // Send the overlay being left if it has an edit waiting, then load the new one.
    flushPendingSave();
    setNotice(null);
    setActiveIdx(idx);
    applyProfile(profiles[idx]);
  };

  const uniqueProfileName = (base: string): string => {
    const names = new Set(profiles.map((p) => p.name.toLowerCase()));
    if (!names.has(base.toLowerCase())) return base;
    for (let n = 2; ; n++) if (!names.has(`${base} ${n}`.toLowerCase())) return `${base} ${n}`;
  };

  // New = a fresh default-styled overlay; duplicate = a copy of the current
  // one. Both keep the current sources (the point of multiple overlays is the
  // same chat in different styles). Either way it is a draft on this device
  // until its first publish mints its own link.
  const addProfile = (duplicate: boolean) => {
    flushPendingSave();
    setNotice(null);
    const p: OverlayProfile = {
      uid: newProfileUid(),
      name: uniqueProfileName(duplicate ? `${profiles[activeIdx].name} copy` : `Overlay ${profiles.length + 1}`),
      id: null,
      version: null,
      style: duplicate ? { ...style } : { ...DEFAULT_OVERLAY_STYLE },
      sources: [...sources],
    };
    const idx = profiles.length;
    setProfiles((list) => [...list, p]);
    setActiveIdx(idx);
    applyProfile(p);
  };

  const commitRename = () => {
    const name = renameValue.trim();
    setRenaming(false);
    if (!name || name === profiles[activeIdx].name) return;
    setProfiles((list) => list.map((p, i) => (i === activeIdx ? { ...p, name: uniqueProfileName(name) } : p)));
    // The name travels inside the published style; nudge a save.
    if (overlayIdRef.current) setStyle((s) => ({ ...s }));
  };

  const deleteProfile = async () => {
    if (profiles.length <= 1) return;
    // Local state math runs synchronously BEFORE the request below, so a save
    // response landing meanwhile can't be clobbered by a stale list.
    const victim = profiles[activeIdx];
    cancelPendingSave(victim.uid);
    const nextList = profiles.filter((_, i) => i !== activeIdx);
    const nextIdx = Math.max(0, activeIdx - 1);
    setProfiles(nextList);
    setActiveIdx(nextIdx);
    applyProfile(nextList[nextIdx]);
    setNotice(null);
    // Retire the link on the account so OBS stops serving it and the other
    // surfaces drop it too. Offline just leaves it; the next open reconciles.
    if (victim.id) {
      void apiRequest('DELETE', `${PUBLISH_PATH}/${victim.id}`).catch(() => { /* offline */ });
    }
  };

  // ── The account's overlays ───────────────────────────────────────────────
  // On open, and whenever the window comes back into focus (the other surface
  // may have changed something), line the list up with the account: published
  // overlays take the account's copy, ones gone from the account leave the list,
  // overlays made elsewhere appear, and drafts stay as they are.
  const lastSyncRef = useRef(0);
  const syncWithAccount = async () => {
    if (Date.now() - lastSyncRef.current < 10_000) return;
    lastSyncRef.current = Date.now();
    let rows: ServerOverlay[];
    try {
      const res = await apiRequest('GET', PUBLISH_PATH, 'all=1');
      if (!res.ok) return; // offline or signed out: keep what this device has
      rows = res.json<{ overlays?: ServerOverlay[] }>()?.overlays ?? [];
    } catch {
      return;
    }
    // A save in flight or waiting would race the copy about to be loaded; the
    // next focus picks the account's copy up instead.
    if (pendingSaveRef.current || inFlightRef.current.size > 0) return;
    const current = profilesRef.current;
    const { profiles: next, dropped } = reconcileProfiles(current, rows);
    const unchanged =
      next.length === current.length &&
      next.every((p, i) => p === current[i] || (p.uid === current[i].uid && p.version === current[i].version && p.id === current[i].id));
    if (unchanged) return;
    for (const p of next) versionsRef.current.set(p.uid, p.version);
    const activeUid = activeUidRef.current;
    const idx = Math.max(0, next.findIndex((p) => p.uid === activeUid));
    setProfiles(next);
    setActiveIdx(idx);
    const before = current.find((p) => p.uid === next[idx].uid);
    if (!before || before.version !== next[idx].version || before.id !== next[idx].id) applyProfile(next[idx]);
    if (dropped.length) {
      setNotice(
        dropped.length === 1
          ? `"${dropped[0]}" is no longer on your account, so it left this list.`
          : `${dropped.length} overlays are no longer on your account, so they left this list.`,
      );
    }
  };
  const syncRef = useRef(syncWithAccount);
  useEffect(() => {
    syncRef.current = syncWithAccount;
  });
  useEffect(() => {
    void syncRef.current();
    const onFocus = () => void syncRef.current();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, []);

  const fontOptions = useMemo(
    () => [
      // Preview each font in its own typeface ("Ag") so the difference is visible.
      ...FONT_OPTIONS.map((f) => ({
        value: f.value,
        label: f.label,
        icon: <span style={{ fontFamily: f.value, fontSize: 15, lineHeight: 1, width: 24, display: 'inline-block', textAlign: 'center' }}>Ag</span>,
      })),
      { value: CUSTOM_FONT, label: 'Custom…' },
    ],
    [],
  );
  const isCustomFont = !FONT_OPTIONS.some((f) => f.value === style.fontFamily);
  // Distinct platforms currently added as sources — drives the per-platform event
  // toggles + shows the Super Chat currency picker only when YouTube is present.
  const sourceProviders = useMemo(
    () => Array.from(new Set(sources.map((s) => s.provider))),
    [sources],
  );
  // Which platforms get their own event-filter group: the added sources, or all
  // four when nothing's added yet so the panel isn't empty while designing.
  const eventProviders = useMemo(
    () => (sourceProviders.length ? sourceProviders : SOURCE_PROVIDERS)
      .filter((p) => (PROVIDER_EVENT_CATEGORIES[p] ?? []).length > 0),
    [sourceProviders],
  );
  // Platform-specific first (Bits on Twitch, Super Chats on YouTube), generic after.
  const catLabel = (provider: ProviderId, id: string) =>
    PROVIDER_CATEGORY_LABELS[provider]?.[id as EventCategory]
      ?? EVENT_CATEGORIES.find((c) => c.id === id)?.label
      ?? id;
  const currencyOptions = useMemo(
    () => [{ value: '', label: 'As sent' }, ...CURRENCY_OPTIONS.map((c) => ({ value: c, label: c }))],
    [],
  );
  // Each emoji-style option previews a few sample emoji in that style (or the OS
  // font for 'system') so the difference is visible before picking.
  const emojiStyleOptions = useMemo(
    () => EMOJI_STYLES.map((e) => ({
      value: e.value,
      label: e.label,
      icon: (
        <span className="inline-flex items-center gap-0.5">
          {EMOJI_SAMPLES.map((s) => (e.value === 'system'
            ? <span key={s.cp} style={{ fontSize: 18, lineHeight: 1 }}>{s.char}</span>
            : <img
                key={s.cp}
                src={e.value === 'twitter'
                  ? `https://cdn.jsdelivr.net/gh/jdecked/twemoji@15.1.0/assets/svg/${s.cp}.svg`
                  : `https://cdn.jsdelivr.net/npm/emoji-datasource-${e.value}@15.1.2/img/${e.value}/64/${s.cp}.png`}
                alt=""
                width={18}
                height={18}
                loading="lazy"
                style={{ display: 'inline-block' }}
              />
          ))}
        </span>
      ),
    })),
    [],
  );

  return (
    <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(340px,430px)]">
      {/* ── Controls ─────────────────────────────────────────────── */}
      <div className="space-y-5 min-w-0">
        {/* Which overlay you are editing, and the two things you do to it.
            Sticks against the settings dialog's scroll port so switching
            overlays never means scrolling back to the top; data-settings-sticky
            lets the dialog's deep-link scroll math subtract its height.

            A card, deliberately, and the reason is in `.settings-pinned-card`:
            a veil that dissolves downward has no edge, so rows do not go UNDER
            it, they half-dissolve INSIDE it, which is what made this the
            ugliest thing in the dialog. A card has a rim, so content simply
            ends at it. Rounded to match the settings cards it sits above,
            because the rest of this tab is a stack of exactly those and a
            full-bleed band would be a second grammar. */}
        {/* The host page can push the pinned parts down past its own sticky
            header with --overlay-builder-sticky-top; the app leaves the default. */}
        <div data-settings-sticky className="sticky z-20" style={{ top: 'var(--overlay-builder-sticky-top, 0.25rem)' }}>
          <div className="settings-pinned-card space-y-2.5 px-3 py-2.5">
        {/* Profiles: each is its own published overlay (own OBS link + style +
            sources). A compact inline cluster — the picker sizes to its content
            and the actions are small icon buttons beside it. */}
        {/* flex-wrap: in a narrow column the Reset/Publish cluster drops to
            its own line (still right-aligned via ml-auto) instead of running
            out of the column into the preview. */}
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-[12px] text-textMuted mr-0.5">Overlay</span>
          {renaming ? (
            <input
              autoFocus
              value={renameValue}
              onChange={(e) => setRenameValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') { e.preventDefault(); commitRename(); }
                if (e.key === 'Escape') setRenaming(false);
              }}
              onBlur={commitRename}
              className="w-[200px] rounded-lg bg-glass border border-borderLight px-2.5 py-1 text-[13px] text-textPrimary focus:outline-none focus:border-accent/60"
            />
          ) : (
            <Dropdown
              value={String(activeIdx)}
              options={profiles.map((p, i) => ({ value: String(i), label: p.name }))}
              onChange={(v) => switchProfile(parseInt(v, 10))}
              className="max-w-[200px]"
            />
          )}
          {renaming ? (
            <Tooltip content="Save name">
              <button onClick={commitRename} className="glass-button rounded-md p-1.5 text-textSecondary hover:text-textPrimary transition-colors">
                <Check size={13} />
              </button>
            </Tooltip>
          ) : (
            <Tooltip content="Rename">
              <button
                onClick={() => { setRenameValue(profiles[activeIdx]?.name ?? ''); setRenaming(true); }}
                className="glass-button rounded-md p-1.5 text-textSecondary hover:text-textPrimary transition-colors"
              >
                <Pencil size={13} />
              </button>
            </Tooltip>
          )}
          <Tooltip content="New overlay">
            <button onClick={() => addProfile(false)} className="glass-button rounded-md p-1.5 text-textSecondary hover:text-textPrimary transition-colors">
              <Plus size={13} />
            </button>
          </Tooltip>
          <Tooltip content="Duplicate">
            <button onClick={() => addProfile(true)} className="glass-button rounded-md p-1.5 text-textSecondary hover:text-textPrimary transition-colors">
              <Copy size={13} />
            </button>
          </Tooltip>
          {profiles.length > 1 && (
            <Tooltip content={confirmDelete ? 'Click again to delete and retire its link' : 'Delete'}>
              <button
                onClick={() => {
                  if (!confirmDelete) { setConfirmDelete(true); return; }
                  setConfirmDelete(false);
                  void deleteProfile();
                }}
                onBlur={() => setConfirmDelete(false)}
                className={`glass-button rounded-md p-1.5 transition-colors ${confirmDelete ? 'text-error' : 'text-textSecondary hover:text-error'}`}
              >
                <Trash2 size={13} />
              </button>
            </Tooltip>
          )}
          {/* Publish/copy sits with the picker rather than at the foot of the
              page: it is the one action you return to after every tweak, and at
              the bottom of whichever tab you happened to be on it read as
              buried. The OBS size reminder follows on its own line below. */}
          <div className="ml-auto flex items-center gap-1.5">
            {/* Two-step, because this throws away every setting on the overlay
                and the first click used to do it outright. Arming inline keeps
                it one gesture away without a dialog to dismiss; it disarms
                itself so a stray click never leaves a live trigger under the
                cursor. */}
            <Tooltip content={resetArmed ? 'This clears every setting on this overlay' : 'Reset this overlay to defaults'}>
              <button
                onClick={() => {
                  if (!resetArmed) { setResetArmed(true); return; }
                  setResetArmed(false);
                  setStyle({ ...DEFAULT_OVERLAY_STYLE });
                }}
                className={`inline-flex items-center gap-1.5 rounded-md px-2 py-1.5 text-[12px] transition-colors flex-shrink-0 ${
                  resetArmed ? 'text-error' : 'text-textMuted hover:text-textPrimary'
                }`}
              >
                <RotateCcw size={13} /> {resetArmed ? 'Reset everything?' : 'Reset'}
              </button>
            </Tooltip>
            <Tooltip
              content={
                sources.length === 0
                  ? 'Add a source first'
                  : publishedUrl
                    ? 'Copy the OBS link again. It stays in sync as you tweak, so you never need to re-copy.'
                    : `Publish once to get ${profiles.length > 1 ? 'this overlay its own' : 'a permanent'} OBS Browser Source link.`
              }
            >
              <button
                onClick={publish}
                disabled={publishState === 'publishing' || sources.length === 0}
                // Accent on GLASS, not a filled accent slab. Publish does need to
                // outrank the four icon buttons beside it (it is the action you
                // come back to after every tweak, and as a plain glass button it
                // looked exactly like Duplicate) — but a solid fill is toast and
                // banner language, and dropping one into a glass settings pane
                // reads as brighter than anything else on the surface. The glass
                // body keeps it in the material and the accent label carries the
                // rank, which is how BadgesOverlay marks its selected chips.
                // `.glass-button:hover` already lifts the body, so no hover here.
                className="glass-button inline-flex flex-shrink-0 items-center gap-1.5 rounded-md px-2.5 py-1.5 text-[12px] font-medium text-accent disabled:cursor-not-allowed disabled:opacity-50"
              >
                <Link2 size={13} />{' '}
                {publishState === 'publishing'
                  ? 'Publishing…'
                  : publishState === 'done'
                    ? 'Copied'
                    : publishedUrl
                      ? 'Copy overlay URL'
                      : 'Publish overlay URL'}
              </button>
            </Tooltip>
          </div>
        </div>

        {/* One status line under the publish controls: a publish error, or the OBS
            source size once there is a link to paste. The size is the one
            piece of setup people get wrong and it names THEIR layout, so the
            number stays visible; the why is a hover away. */}
        {publishState === 'error' ? (
          <p className="flex items-center gap-1.5 text-[12px] leading-snug text-error">
            <AlertTriangle size={13} className="flex-shrink-0" />
            <span>{publishError}</span>
          </p>
        ) : notice ? (
          <p className="flex items-center gap-1.5 text-[12px] leading-snug text-textSecondary">
            <RefreshCw size={13} className="flex-shrink-0 text-accent" />
            <span className="min-w-0 flex-1">{notice}</span>
            <button onClick={() => setNotice(null)} aria-label="Dismiss" className="flex-shrink-0 text-textMuted hover:text-textPrimary transition-colors">
              <X size={13} />
            </button>
          </p>
        ) : publishedUrl ? (
          <Tooltip content="OBS crops to the Browser Source size and never grows to fit, so it has to match your Layout size exactly.">
            <p className="inline-flex cursor-help items-center gap-1.5 text-[12px] leading-snug text-textMuted">
              <AlertTriangle size={13} className="flex-shrink-0 text-warning" />
              <span>
                OBS Browser Source size:{' '}
                <span className="font-semibold tabular-nums text-textSecondary">{style.width} × {style.height}</span>
              </span>
            </p>
          </Tooltip>
        ) : null}

          </div>
        </div>

        <SettingsSection label="Sources" description="The channels this overlay reads chat from, on any mix of platforms.">
          <div className="settings-row -mx-4 px-4 py-3 space-y-2.5">
            <label className="block text-[11px] text-textSecondary">Add a channel</label>
            <div className="flex items-center gap-2">
              <Dropdown
                value={addProvider}
                options={SOURCE_PROVIDERS.map((p) => ({ value: p, label: PROVIDERS[p].label, icon: <ProviderIcon provider={p} size="14px" /> }))}
                onChange={(v) => { setAddProvider(v); setAddError(null); }}
                className="flex-shrink-0"
              />
              <input
                value={addChannel}
                onChange={(e) => { setAddChannel(e.target.value); if (addError) setAddError(null); }}
                onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); void addSource(); } }}
                placeholder={SOURCE_PLACEHOLDER[addProvider]}
                className="flex-1 min-w-0 rounded-lg bg-glass border border-borderLight px-3 py-1.5 text-sm text-textPrimary placeholder:text-textMuted focus:outline-none focus:border-accent/60"
              />
              <button onClick={() => void addSource()} disabled={addBusy} className="inline-flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-sm font-medium glass-input text-textPrimary flex-shrink-0 disabled:opacity-50">
                <Plus size={14} /> Add
              </button>
            </div>
            {addError && <p className="text-[12px] text-error">{addError}</p>}
            {sources.length === 0 ? (
              <p className="text-[12px] text-textMuted">No sources yet. Add a channel to preview its live chat.</p>
            ) : (
              <div className="flex flex-col gap-1.5">
                {sources.map((s) => (
                  <SourceRow key={`${s.provider}:${s.channel}`} source={s} onRemove={() => removeSource(s)} />
                ))}
              </div>
            )}
            <p className="text-[12px] leading-relaxed text-textMuted">
              Paste a channel link from any platform and it is added to that platform.
            </p>
          </div>
          <SettingsRow onReset={resetFor('sources')} title="Platform filter" description="Hide a platform's messages without removing its source.">
            <div className="flex flex-wrap gap-2">
              {SOURCE_PROVIDERS.map((id) => {
                // Only a platform you've actually added as a source can be toggled;
                // the rest gray out (nothing to show or hide for them).
                const hasSource = sourceProviders.includes(id);
                const active = hasSource && style.sources.includes(id);
                return (
                  <button
                    key={id}
                    onClick={() => hasSource && toggleSourceFilter(id)}
                    disabled={!hasSource}
                    style={{ borderRadius: 8 }}
                    className={`inline-flex items-center gap-1.5 px-2.5 py-1.5 text-[13px] font-medium transition-all disabled:opacity-40 disabled:cursor-not-allowed ${active ? 'glass-input text-textPrimary' : 'glass-button text-textSecondary hover:text-textPrimary'}`}
                  >
                    <ProviderIcon provider={id} size="15px" />
                    {PROVIDERS[id].label}
                  </button>
                );
              })}
            </div>
          </SettingsRow>
          <SettingsRow onReset={resetFor('sourceTag')} title="Source tag" description="Shows which platform each message came from, as a dot, an icon, or the platform name.">
            <SegmentedSelect
              value={style.sourceTag}
              onChange={(v) => set('sourceTag', v)}
              options={[
                { value: 'none', label: 'Off' },
                { value: 'dot', label: 'Dot' },
                { value: 'icon', label: 'Icon' },
                { value: 'label', label: 'Label' },
              ]}
            />
          </SettingsRow>
        </SettingsSection>

        <SettingsSection label="Layout" description="The overlay's size and background; set your OBS Browser Source to the same width and height.">
          <SettingsRow title="Presets" description="Common sizes to start from, then fine-tune below.">
            <div className="flex flex-wrap gap-2">
              {SIZE_PRESETS.map((p) => {
                const active = style.width === p.width && style.height === p.height;
                return (
                  <button
                    key={p.label}
                    onClick={() => setStyle((s) => ({ ...s, width: p.width, height: p.height }))}
                    style={{ borderRadius: 8 }}
                    className={`px-2.5 py-1.5 text-[13px] font-medium transition-all ${active ? 'glass-input text-textPrimary' : 'glass-button text-textSecondary hover:text-textPrimary'}`}
                  >
                    {p.label}
                  </button>
                );
              })}
            </div>
          </SettingsRow>
          <SettingsRow onReset={resetFor('width')} title="Width" description="How wide the overlay is; long messages wrap sooner in a narrow one.">
            <Slider value={style.width} min={OVERLAY_LIMITS.width.min} max={OVERLAY_LIMITS.width.max} step={10} onChange={(v) => set('width', Math.round(v))} format={(v) => `${v}px`} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('height')} title="Height" description="Taller fits more chat on screen at once.">
            <Slider value={style.height} min={OVERLAY_LIMITS.height.min} max={OVERLAY_LIMITS.height.max} step={10} onChange={(v) => set('height', Math.round(v))} format={(v) => `${v}px`} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('background')} title="Background" description="Transparent lets your scene show through. Solid draws a panel behind the chat.">
            <SegmentedSelect
              value={style.background}
              onChange={(v) => set('background', v)}
              options={[{ value: 'transparent', label: 'Transparent' }, { value: 'solid', label: 'Solid' }]}
            />
          </SettingsRow>
          {style.background === 'solid' && (
            <SettingsSubGroup>
              <SettingsRow onReset={resetFor('backgroundColor')} title="Background color" control={
                <input type="color" value={style.backgroundColor} onChange={(e) => set('backgroundColor', e.target.value)} className="h-7 w-10 rounded cursor-pointer bg-transparent border border-borderSubtle" />
              } />
              <SettingsRow onReset={resetFor('backgroundOpacity')} title="Background opacity">
                <Slider value={style.backgroundOpacity} min={0} max={1} step={0.05} onChange={(v) => set('backgroundOpacity', v)} format={(v) => `${Math.round(v * 100)}%`} />
              </SettingsRow>
            </SettingsSubGroup>
          )}
        </SettingsSection>

        <SettingsSection label="Text" description="Font, sizing, and legibility of the message text.">
          <SettingsRow onReset={resetFor('fontFamily')} title="Font" control={
            <Dropdown
              value={isCustomFont ? CUSTOM_FONT : style.fontFamily}
              options={fontOptions}
              onChange={(v) => set('fontFamily', v === CUSTOM_FONT ? CUSTOM_FONT_STARTER : v)}
              align="right"
            />
          } />
          {isCustomFont && (
            <SettingsSubGroup>
            <SettingsRow onReset={resetFor('fontFamily')} title="Custom font" description="Loads automatically, here and on your overlay.">
              <div className="w-full space-y-2">
                <input
                  value={primaryFamilyName(style.fontFamily)}
                  onChange={(e) => set('fontFamily', `'${e.target.value.replace(/['"]/g, '')}', sans-serif`)}
                  placeholder="e.g. Poppins"
                  style={{ fontFamily: style.fontFamily }}
                  className="w-full min-w-0 rounded-lg bg-glass border border-borderLight px-3 py-1.5 text-sm text-textPrimary placeholder:text-textMuted focus:outline-none focus:border-accent/60"
                />
                <p className="text-[12px] leading-relaxed text-textMuted">
                  Any free font from fonts.google.com works, just type its exact name. Fonts installed on your streaming PC work too.
                </p>
              </div>
            </SettingsRow>
            </SettingsSubGroup>
          )}
          <SettingsRow onReset={resetFor('fontSize')} title="Font size">
            <Slider value={style.fontSize} min={OVERLAY_LIMITS.fontSize.min} max={OVERLAY_LIMITS.fontSize.max} onChange={(v) => set('fontSize', v)} format={(v) => `${v}px`} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('lineHeight')} title="Line height" description="Spacing within a wrapped message.">
            <Slider value={style.lineHeight} min={OVERLAY_LIMITS.lineHeight.min} max={OVERLAY_LIMITS.lineHeight.max} step={0.05} onChange={(v) => set('lineHeight', v)} format={(v) => v.toFixed(2)} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('messageGap')} title="Message spacing" description="Gap between messages.">
            <Slider value={style.messageGap} min={OVERLAY_LIMITS.messageGap.min} max={OVERLAY_LIMITS.messageGap.max} onChange={(v) => set('messageGap', v)} format={(v) => `${v}px`} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('textAlign')} title="Text alignment" description="Left, center, or right; event cards line up the same way.">
            <SegmentedSelect value={style.textAlign ?? 'left'} onChange={(v) => set('textAlign', v)} options={OVERLAY_TEXT_ALIGNS} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('fontWeight')} title="Text weight" description="How heavy the text is. Usernames stay bold either way.">
            <SegmentedSelect value={String(style.fontWeight ?? 400)} onChange={(v) => set('fontWeight', parseInt(v, 10))} options={OVERLAY_TEXT_WEIGHTS} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('textItalic')} title="Italic" description="Slant message text. Actions (/me) are italic either way." control={<Toggle enabled={style.textItalic === true} onChange={() => set('textItalic', style.textItalic !== true)} />} />
          <SettingsRow onReset={resetFor('textStrikethrough')} title="Strikethrough" description="Draw a line through message text." control={<Toggle enabled={style.textStrikethrough === true} onChange={() => set('textStrikethrough', style.textStrikethrough !== true)} />} />
          <SettingsRow onReset={resetFor('bodyTextColor')} title="Text color" control={
            <input type="color" value={style.bodyTextColor} onChange={(e) => set('bodyTextColor', e.target.value)} className="h-7 w-10 rounded cursor-pointer bg-transparent border border-borderSubtle" />
          } />
          <SettingsRow onReset={resetFor('textShadow')} title="Text shadow" description="An outline behind text so it stays readable over any scene." control={<Toggle enabled={style.textShadow} onChange={() => set('textShadow', !style.textShadow)} />} />
          <SettingsSubGroup>
            <SettingsRow onReset={resetFor('textShadowColor')} title="Shadow color" disabled={!style.textShadow} control={
              <input type="color" value={style.textShadowColor || '#000000'} onChange={(e) => set('textShadowColor', e.target.value)} disabled={!style.textShadow} className="h-7 w-10 rounded cursor-pointer bg-transparent border border-borderSubtle disabled:cursor-not-allowed" />
            } />
            <SettingsRow onReset={resetFor('textShadowSize')} title="Shadow size" description="How far the shadow spreads. 0 turns it off." disabled={!style.textShadow}>
              <Slider value={style.textShadowSize ?? 2} min={OVERLAY_LIMITS.textShadowSize.min} max={OVERLAY_LIMITS.textShadowSize.max} step={0.5} onChange={(v) => set('textShadowSize', v)} format={(v) => `${v}px`} />
            </SettingsRow>
            <SettingsRow onReset={resetFor('textShadowOpacity')} title="Shadow strength" description="How solid the shadow is." disabled={!style.textShadow}>
              <Slider value={style.textShadowOpacity ?? 0.85} min={OVERLAY_LIMITS.textShadowOpacity.min} max={OVERLAY_LIMITS.textShadowOpacity.max} step={0.05} onChange={(v) => set('textShadowOpacity', v)} format={(v) => `${Math.round(v * 100)}%`} />
            </SettingsRow>
          </SettingsSubGroup>
          <SettingsRow onReset={resetFor('emojiStyle')} title="Emoji style" description="One consistent emoji set across every platform." help="System uses your machine's own emoji font instead." control={<Dropdown value={style.emojiStyle} options={emojiStyleOptions} onChange={(v) => set('emojiStyle', v)} align="right" />} />
        </SettingsSection>

        <SettingsSection label="Emotes & badges" description="Emote sizing and every badge type.">
          <SettingsRow onReset={resetFor('emoteScale')} title="Emote size">
            <Slider value={style.emoteScale} min={OVERLAY_LIMITS.emoteScale.min} max={OVERLAY_LIMITS.emoteScale.max} step={0.05} onChange={(v) => set('emoteScale', v)} format={(v) => `${v.toFixed(2)}x`} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('giantEmotes')} title="Giant emotes" description="The Gigantify an Emote power-up, drawn at 4x like Twitch does." help="The last emote of a gigantified message renders at 4x below the message." control={<Toggle enabled={style.giantEmotes !== false} onChange={() => set('giantEmotes', style.giantEmotes === false)} />} />
          <SettingsSubGroup>
            <SettingsRow onReset={resetFor('giantEmoteAlign')} title="Giant emote placement" description="Where the big emote sits." help="Left, Center, and Right give it its own line below the message. Inline leaves it where it was typed, so an emote-only message shows it right after the name." disabled={style.giantEmotes === false}>
              <SegmentedSelect value={style.giantEmoteAlign ?? 'center'} onChange={(v) => set('giantEmoteAlign', v)} options={GIANT_EMOTE_ALIGNS} />
            </SettingsRow>
          </SettingsSubGroup>
          <SettingsRow onReset={resetFor('showGifs')} title="Chat GIFs" titleBadge={<SourceScope sources={['twitch']} />} description="GIFs that Tier 2 and Tier 3 subscribers post in chat, drawn big like a gigantified emote." help="Follows Giant emote placement: Left, Center and Right give each GIF its own line below the message, Inline leaves it where it was typed. Off shows the short description Twitch sends in its place." control={<Toggle enabled={style.showGifs !== false} onChange={() => set('showGifs', style.showGifs === false)} />} />
          <SettingsRow onReset={resetFor('showPersonalEmotes')} title="7TV personal emotes" titleBadge={<SourceScope sources={['twitch']} />} description="Emotes a 7TV subscriber brings into every channel." help="A subscriber's personal 7TV set works in every channel, so chatters can show emotes your channel never added. Off renders those as the word that was typed. Your channel's own 7TV emotes are unaffected." control={<Toggle enabled={style.showPersonalEmotes !== false} onChange={() => set('showPersonalEmotes', style.showPersonalEmotes === false)} />} />
          <SettingsRow onReset={resetFor('showBadges')} title="Show badges" description="Badges the platform sends: subscriber, moderator, VIP, and the rest." help="These arrive with each message from Twitch, Kick, YouTube and TikTok. Chat-client badges and the StreamNook member badge are separate, on the Third-party badges switch below, so turning this off leaves those showing." control={<Toggle enabled={style.showBadges} onChange={() => set('showBadges', !style.showBadges)} />} />
          {/* Scales every badge in the row, not just the platform ones, so this
              only goes dead when BOTH badge switches are off. */}
          <SettingsRow onReset={resetFor('badgeScale')} title="Badge size" disabled={!style.showBadges && style.showThirdPartyBadges === false}>
            <Slider value={style.badgeScale} min={OVERLAY_LIMITS.badgeScale.min} max={OVERLAY_LIMITS.badgeScale.max} step={0.05} onChange={(v) => set('badgeScale', v)} format={(v) => `${v.toFixed(2)}x`} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('showThirdPartyBadges')} title="Third-party badges" description="7TV, FFZ, Chatterino, and more." help="Native platform badges follow the Show badges toggle above." control={<Toggle enabled={style.showThirdPartyBadges} onChange={() => set('showThirdPartyBadges', !style.showThirdPartyBadges)} />} />
          <SettingsSubGroup>
          <SettingsRow onReset={resetFor('hiddenBadgeProviders')} title="Badge providers" description="Pick which providers show." help="StreamNook is the member badge. The rest are third-party.">
            <div className="flex flex-wrap gap-2">
              {THIRD_PARTY_BADGE_PROVIDERS.map((p) => {
                const on = style.showThirdPartyBadges !== false && !(style.hiddenBadgeProviders ?? []).includes(p.id);
                return (
                  <button
                    key={p.id}
                    onClick={() => toggleBadgeProvider(p.id)}
                    disabled={style.showThirdPartyBadges === false}
                    style={{ borderRadius: 8 }}
                    className={`px-2.5 py-1.5 text-[13px] font-medium transition-all disabled:opacity-40 disabled:cursor-not-allowed ${on ? 'glass-input text-textPrimary' : 'glass-button text-textSecondary hover:text-textPrimary'}`}
                  >
                    {p.label}
                  </button>
                );
              })}
            </div>
          </SettingsRow>
          </SettingsSubGroup>
        </SettingsSection>

        <SettingsSection label="Chatters" description="Picture, name, and cosmetics of the person behind each message.">
          <SettingsRow onReset={resetFor('showAvatars')} title="Profile pictures" titleBadge={<SourceScope sources={['youtube', 'tiktok']} />} description="Avatars beside names." help="YouTube and TikTok send avatars. Twitch and Kick don't have them, so nothing changes there." control={<Toggle enabled={style.showAvatars} onChange={() => set('showAvatars', !style.showAvatars)} />} />
          <SettingsRow onReset={resetFor('showAtSign')} title="@ before usernames" titleBadge={<SourceScope sources={['youtube']} />} description="Keep the @ on YouTube handles." help="YouTube names arrive as @handles. Off drops the leading @ from every name." control={<Toggle enabled={style.showAtSign} onChange={() => set('showAtSign', !style.showAtSign)} />} />
          <SettingsRow onReset={resetFor('readableNameColors')} title="Readable name colors" description="Brighten names too dark to read." help="Walks a chatter's own color lighter until it stands out against your bubbles, your background, or the stream. The hue stays the same, and 7TV paints are never changed." control={<Toggle enabled={style.readableNameColors} onChange={() => set('readableNameColors', !style.readableNameColors)} />} />
          <SettingsRow onReset={resetFor('showPaints')}
            title={<span className="inline-flex items-center gap-1.5"><SevenTVLogo className="h-[11px] w-auto text-[#29b6f6]" /> Paints</span>}
            description="Colored and animated username gradients."
            control={<Toggle enabled={style.showPaints} onChange={() => set('showPaints', !style.showPaints)} />}
          />
          <SettingsRow onReset={resetFor('showAtmospheres')}
            title={<span className="inline-flex items-center gap-1.5"><img src={streamNookLogo} alt="" className="w-4 h-4 object-contain" draggable={false} /> Atmospheres</span>}
            description="A member's animated wash behind their own messages." help="Separate from event styles and your overlay's background."
            control={<Toggle enabled={style.showAtmospheres} onChange={() => set('showAtmospheres', !style.showAtmospheres)} />}
          />
          <SettingsRow onReset={resetFor('firstTimeStyle')} title="First-time chatters" titleBadge={<SourceScope sources={['twitch']} />} description="Mark someone's first-ever message in the channel." help="Twitch draws the outline and label Twitch chat uses. StreamNook uses the app chat's purple highlight. Only Twitch sends the signal, so it never fires on other platforms.">
            <SegmentedSelect
              value={style.firstTimeStyle}
              onChange={(v) => set('firstTimeStyle', v)}
              options={[{ value: 'off', label: 'Off' }, { value: 'twitch', label: 'Twitch' }, { value: 'streamnook', label: 'StreamNook' }]}
            />
          </SettingsRow>
          <SettingsSubGroup>
          <SettingsRow onReset={resetFor('firstTimeColor')}
            title="Highlight color"
            description="Default matches the style: Twitch pink or StreamNook purple." help="One color drives the outline, fill, bar, and label together."
            disabled={style.firstTimeStyle === 'off'}
            control={
              <div className="flex items-center gap-2">
                {!!style.firstTimeColor && (
                  <button onClick={() => set('firstTimeColor', '')} className="text-[12px] text-textSecondary hover:text-textPrimary">
                    Default
                  </button>
                )}
                <input
                  type="color"
                  value={style.firstTimeColor || (style.firstTimeStyle === 'streamnook' ? '#a855f7' : '#ff38db')}
                  onChange={(e) => set('firstTimeColor', e.target.value)}
                  disabled={style.firstTimeStyle === 'off'}
                  className="h-7 w-10 rounded cursor-pointer bg-transparent border border-borderSubtle disabled:cursor-not-allowed"
                />
              </div>
            }
          />
          <SettingsRow onReset={resetFor('firstTimeFill')} title="Fill the highlight" description="A faint tint inside the outline." help="Color-matched to the outline, so the message reads highlighted instead of just bordered. The StreamNook style has its own wash." disabled={style.firstTimeStyle !== 'twitch'} control={<Toggle enabled={style.firstTimeFill} onChange={() => set('firstTimeFill', !style.firstTimeFill)} />} />
          <SettingsRow onReset={resetFor('firstTimeAnimation')} title="Animation" description="Plays on the border when the message lands." help="Sheen sweeps a glint across it. Pulse breathes it brighter. Chase sends a spark around it." disabled={style.firstTimeStyle === 'off'}>
            <SegmentedSelect
              value={style.firstTimeAnimation}
              onChange={(v) => set('firstTimeAnimation', v)}
              options={OVERLAY_ANIMATIONS.map((a) => ({ value: a.value, label: a.label }))}
            />
          </SettingsRow>
          <SettingsRow onReset={resetFor('firstTimeAnimateRepeat')} title="Repeat the animation" description="Keep it going while the message is on screen." help="Sheen and Pulse replay every 5 seconds. Chase spins continuously." disabled={style.firstTimeStyle === 'off' || style.firstTimeAnimation === 'none'} control={<Toggle enabled={style.firstTimeAnimateRepeat} onChange={() => set('firstTimeAnimateRepeat', !style.firstTimeAnimateRepeat)} />} />
          </SettingsSubGroup>
        </SettingsSection>

        <SettingsSection label="Messages" description="How messages render and flow.">
          <SettingsRow onReset={resetFor('replyStyle')} title="Replies" description="How a reply shows the message it answers." help={'Context line shows "Replying to @name: their message" above it. @username puts just the name in front of the message, the way Twitch chat did before threading. Off shows the message on its own.'}>
            <SegmentedSelect value={style.replyStyle} options={REPLY_STYLES} onChange={(v) => set('replyStyle', v)} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('linkStyle')} title="Links" description="Accent gives links their own color. Body text leaves them as the rest of the message.">
            <SegmentedSelect value={style.linkStyle} options={LINK_STYLES} onChange={(v) => set('linkStyle', v)} />
          </SettingsRow>
          <SettingsSubGroup>
            <SettingsRow onReset={resetFor('linkColor')} title="Link color" disabled={style.linkStyle !== 'accent'} control={
              <input type="color" value={style.linkColor || DEFAULT_LINK_COLOR} onChange={(e) => set('linkColor', e.target.value)} disabled={style.linkStyle !== 'accent'} className="h-7 w-10 rounded cursor-pointer bg-transparent border border-borderSubtle disabled:cursor-not-allowed" />
            } />
            <SettingsRow onReset={resetFor('linkUnderline')} title="Underline links" control={<Toggle enabled={style.linkUnderline !== false} onChange={() => set('linkUnderline', style.linkUnderline === false)} />} />
          </SettingsSubGroup>
          <SettingsRow onReset={resetFor('showTimestamps')} title="Show timestamps" control={<Toggle enabled={style.showTimestamps} onChange={() => set('showTimestamps', !style.showTimestamps)} />} />
          <SettingsRow onReset={resetFor('bubble')} title="Message bubbles" description="Each message in its own bubble. Reads better over busy gameplay." help="A member's atmosphere replaces the bubble on their rows." control={<Toggle enabled={style.bubble} onChange={() => set('bubble', !style.bubble)} />} />
          {style.bubble && (
            <SettingsSubGroup>
              <SettingsRow onReset={resetFor('bubbleShape')} title="Bubble shape" description="Rounded, pill, or speech bubble." help="Rounded uses the corner radius below. Pill fully rounds the ends. Speech tucks in the bottom-left corner like a messenger bubble.">
                <SegmentedSelect
                  value={style.bubbleShape}
                  onChange={(v) => set('bubbleShape', v)}
                  options={BUBBLE_SHAPES.map((b) => ({ value: b.value, label: b.label }))}
                />
              </SettingsRow>
              <SettingsRow onReset={resetFor('bubbleRadius')} title="Corner radius" disabled={style.bubbleShape === 'pill'}>
                <Slider value={style.bubbleRadius} min={OVERLAY_LIMITS.bubbleRadius.min} max={OVERLAY_LIMITS.bubbleRadius.max} step={1} onChange={(v) => set('bubbleRadius', Math.round(v))} format={(v) => `${v}px`} />
              </SettingsRow>
              <SettingsRow onReset={resetFor('bubbleColor')} title="Bubble color" control={
                <input type="color" value={style.bubbleColor} onChange={(e) => set('bubbleColor', e.target.value)} className="h-7 w-10 rounded cursor-pointer bg-transparent border border-borderSubtle" />
              } />
              <SettingsRow onReset={resetFor('bubbleOpacity')} title="Bubble opacity">
                <Slider value={style.bubbleOpacity} min={OVERLAY_LIMITS.bubbleOpacity.min} max={OVERLAY_LIMITS.bubbleOpacity.max} step={0.05} onChange={(v) => set('bubbleOpacity', v)} format={(v) => `${Math.round(v * 100)}%`} />
              </SettingsRow>
            </SettingsSubGroup>
          )}
          <SettingsRow onReset={resetFor('maxMessageLines')} title="Max lines per message" description="Cut long messages off so one wall of text can't eat the canvas.">
            <Slider value={style.maxMessageLines} min={OVERLAY_LIMITS.maxMessageLines.min} max={OVERLAY_LIMITS.maxMessageLines.max} step={1} onChange={(v) => set('maxMessageLines', Math.round(v))} format={(v) => (v === 0 ? 'No limit' : `${v}`)} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('maxMessageAgeSec')} title="Remove messages after" description="Takes a message off the overlay once it has been up this long, so a quiet stream never shows stale chat.">
            <Slider value={style.maxMessageAgeSec} min={OVERLAY_LIMITS.maxMessageAgeSec.min} max={OVERLAY_LIMITS.maxMessageAgeSec.max} step={5} onChange={(v) => set('maxMessageAgeSec', Math.round(v))} format={(v) => (v === 0 ? 'Never' : `${v}s`)} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('restoreOnReload')} title="Restore chat on reload" description="Bring back the last messages when the OBS source reloads." help="Off means the overlay comes back cleared when you reopen OBS or start a stream." control={<Toggle enabled={style.restoreOnReload} onChange={() => set('restoreOnReload', !style.restoreOnReload)} />} />
          <SettingsRow onReset={resetFor('loadRecentChat')} title="Recent chat on start" titleBadge={<SourceScope sources={['twitch']} />} description="Open with the channel's last messages." help="When the overlay starts, it fills with up to 40 recent Twitch messages instead of waiting for new ones. Deleted messages stay gone." control={<Toggle enabled={style.loadRecentChat} onChange={() => set('loadRecentChat', !style.loadRecentChat)} />} />
          <SettingsRow onReset={resetFor('modCommands')} title="Mod commands" titleBadge={<SourceScope sources={['twitch']} />} description="Mods can reload or clear the overlay from chat." help="The broadcaster and moderators type !refreshoverlay to reload it or !clearoverlay to empty it. The command itself never shows on the overlay." control={<Toggle enabled={style.modCommands} onChange={() => set('modCommands', !style.modCommands)} />} />
          <SettingsRow onReset={resetFor('direction')} title="New messages" description="Where incoming messages appear.">
            <SegmentedSelect
              value={style.direction}
              onChange={(v) => set('direction', v)}
              options={[{ value: 'newBottom', label: 'Bottom' }, { value: 'newTop', label: 'Top' }]}
            />
          </SettingsRow>
          <SettingsRow onReset={resetFor('entrance')} title="Entrance" description="How each new message arrives." help="Slide snaps in from the left. Drift floats in diagonally. Rise springs up. Pop scales up. Stamp slams down and settles.">
            <SegmentedSelect
              value={style.entrance}
              onChange={(v) => set('entrance', v)}
              options={OVERLAY_ENTRANCES.map((e) => ({ value: e.value, label: e.label }))}
            />
          </SettingsRow>
        </SettingsSection>

        <>
        <SettingsSection label="Filters" description="Keep bots and command spam out of the overlay.">
          <SettingsRow onReset={resetFor('hideBots')} title="Hide bot messages" description="Nightbot, StreamElements, other known bots, and anyone with a bot badge." help="Channel bots vary and some slip through. Hide any it misses by name under Hidden accounts." control={<Toggle enabled={style.hideBots} onChange={() => set('hideBots', !style.hideBots)} />} />
          <SettingsRow onReset={resetFor('hideCommands')} title="Hide command messages" description="Keeps chat commands like !title off the overlay; choose which ones below." control={<Toggle enabled={style.hideCommands} onChange={() => set('hideCommands', !style.hideCommands)} />} />
          {style.hideCommands && (
            <SettingsSubGroup>
              <SettingsRow onReset={resetFor('commandFilters')} title="Commands to hide">
                <CommandFilterEditor filters={style.commandFilters ?? []} onAdd={addCommandFilter} onRemove={removeCommandFilter} />
              </SettingsRow>
            </SettingsSubGroup>
          )}
          <SettingsRow onReset={resetFor('hidePhrases')} title="Hide messages containing" description="Words or phrases that keep a message off the overlay." help="Matched anywhere in the message, in any case, whatever channel moderation does. Events are unaffected.">
            <PhraseEditor phrases={style.hidePhrases ?? []} onAdd={addPhrase} onRemove={removePhrase} />
          </SettingsRow>
        </SettingsSection>
        <SettingsSection label="Hidden accounts" description="Hide specific people on each source, by username or display name.">
          {sources.length === 0 ? (
            <p className="py-3 text-[13px] text-textMuted">Add a source first, then hide accounts on it.</p>
          ) : (
            sources.map((s) => (
              <BlockRow
                key={sourceKey(s)}
                source={s}
                blocked={style.blockedUsers?.[sourceKey(s)] ?? []}
                onAddBlocked={(n) => addBlockedUser(s, n)}
                onRemoveBlocked={(n) => removeBlockedUser(s, n)}
              />
            ))
          )}
        </SettingsSection>
        </>

        <SettingsSection label="Events" description="Subs, gifts, raids, and more. How they look, and which ones each source shows.">
          <SettingsRow onReset={resetFor('cheerDisplay')} title="Bits messages" titleBadge={<SourceScope sources={['twitch']} />} description="Show a cheer inline like a normal message, or as an event card like subs and raids.">
            <SegmentedSelect value={style.cheerDisplay ?? 'message'} onChange={(v) => set('cheerDisplay', v)} options={CHEER_DISPLAYS} />
          </SettingsRow>
          <SettingsRow onReset={resetFor('eventStyle')} title="Event style" description="A subtle tint, a thin platform-colored ring, or the StreamNook gradient wash." help="Every style shows the sender's badges and paint name.">
            <SegmentedSelect
              value={style.eventStyle}
              onChange={(v) => set('eventStyle', v)}
              options={[{ value: 'plain', label: 'Plain' }, { value: 'outline', label: 'Outline' }, { value: 'streamnook', label: 'StreamNook' }]}
            />
          </SettingsRow>
          <SettingsSubGroup>
          <SettingsRow onReset={resetFor('eventOutlineColor')}
            title="Outline color"
            description="One fixed ring color for every event. Default gives each event its own platform's color."
            disabled={style.eventStyle !== 'outline'}
            control={
              <div className="flex items-center gap-2">
                {!!style.eventOutlineColor && (
                  <button onClick={() => set('eventOutlineColor', '')} className="text-[12px] text-textSecondary hover:text-textPrimary">
                    Default
                  </button>
                )}
                <input
                  type="color"
                  value={style.eventOutlineColor || '#9147ff'}
                  onChange={(e) => set('eventOutlineColor', e.target.value)}
                  disabled={style.eventStyle !== 'outline'}
                  className="h-7 w-10 rounded cursor-pointer bg-transparent border border-borderSubtle disabled:cursor-not-allowed"
                />
              </div>
            }
          />
          <SettingsRow onReset={resetFor('eventFill')} title="Fill the outline" description="A nearly transparent tint inside the ring, matched to the outline's color." disabled={style.eventStyle !== 'outline'} control={<Toggle enabled={style.eventFill} onChange={() => set('eventFill', !style.eventFill)} />} />
          <SettingsRow onReset={resetFor('eventAnimation')} title="Animation" description="Plays on the ring when the event lands." help="Sheen sweeps a glint across it. Pulse breathes it brighter. Chase sends a spark around it." disabled={style.eventStyle !== 'outline'}>
            <SegmentedSelect
              value={style.eventAnimation}
              onChange={(v) => set('eventAnimation', v)}
              options={OVERLAY_ANIMATIONS.map((a) => ({ value: a.value, label: a.label }))}
            />
          </SettingsRow>
          <SettingsRow onReset={resetFor('eventAnimateRepeat')} title="Repeat the animation" description="Keep it going while the event is on screen." help="Sheen and Pulse replay every 5 seconds. Chase spins continuously." disabled={style.eventStyle !== 'outline' || style.eventAnimation === 'none'} control={<Toggle enabled={style.eventAnimateRepeat} onChange={() => set('eventAnimateRepeat', !style.eventAnimateRepeat)} />} />
          </SettingsSubGroup>
          <SettingsRow
            title="Custom event text"
            onReset={resetFor('eventTemplates')}
            description="Your own wording for each event, with tokens for the details." help="Leave one blank to keep what the platform sends. Click a token to drop it in at the cursor, or open the full list to see everything you can reference."
          >
            <TokenLegend />
          </SettingsRow>
          <SettingsSubGroup>
            {EVENT_CATEGORIES.map((c) => (
              <SettingsRow key={`tpl-${c.id}`} title={c.label}>
                <EventTemplateEditor
                  category={c.id}
                  value={style.eventTemplates?.[c.id] ?? ''}
                  onChange={(next) => setStyle((st) => {
                    const templates = { ...(st.eventTemplates ?? {}) };
                    if (next.trim()) templates[c.id] = next;
                    else delete templates[c.id];
                    return { ...st, eventTemplates: templates };
                  })}
                />
              </SettingsRow>
            ))}
          </SettingsSubGroup>
          <SettingsRow
            title="Show events"
            description="Each platform filters on its own."
            help={sourceProviders.length
              ? "Turn a type off and that platform's version of it never reaches the overlay. The other platforms are untouched."
              : "Add sources and this narrows to just those platforms. Turning a type off hides only that platform's version of it."}
          />
          {eventProviders.map((provider) => (
            <SettingsRow
              key={`pe-${provider}`}
              title={<span className="inline-flex items-center gap-1.5"><ProviderIcon provider={provider} size="14px" /> {PROVIDERS[provider].label}</span>}
              // Scoped to this platform: every source filters on its own, so
              // restoring one must not un-hide what was turned off on another.
              onReset={(style.hiddenProviderEvents ?? []).some((k) => k.startsWith(`${provider}:`))
                ? () => setStyle((st) => ({
                    ...st,
                    hiddenProviderEvents: (st.hiddenProviderEvents ?? []).filter((k) => !k.startsWith(`${provider}:`)),
                  }))
                : undefined}
            >
              <div className="flex flex-wrap gap-2">
                {(PROVIDER_EVENT_CATEGORIES[provider] ?? []).map((cat) => {
                  const key = `${provider}:${cat}`;
                  const on = !(style.hiddenProviderEvents ?? []).includes(key);
                  return (
                    <button
                      key={key}
                      onClick={() => toggleProviderEvent(key)}
                      style={{ borderRadius: 8 }}
                      className={`px-2.5 py-1.5 text-[13px] font-medium transition-all ${on ? 'glass-input text-textPrimary' : 'glass-button text-textSecondary hover:text-textPrimary'}`}
                    >
                      {catLabel(provider, cat)}
                    </button>
                  );
                })}
              </div>
            </SettingsRow>
          ))}
          {sourceProviders.includes('youtube') && (
            <SettingsRow onReset={resetFor('superchatCurrency')}
              title="Super Chat currency"
              description="Convert every YouTube Super Chat into one currency, or show each as it was sent."
              control={<Dropdown value={style.superchatCurrency} options={currencyOptions} onChange={(v) => set('superchatCurrency', v)} align="right" />}
            />
          )}
        </SettingsSection>
      </div>

      {/* ── Preview studio ───────────────────────────────────────── */}
      <div className="lg:sticky self-start space-y-3" style={{ top: 'var(--overlay-builder-sticky-top, 0.5rem)' }}>
        <div className="flex items-center justify-between px-1 gap-3 flex-wrap">
          <div className="flex items-center gap-2">
            <SegmentedSelect
              value={previewMode}
              onChange={setPreviewMode}
              options={[{ value: 'sample', label: 'Sample' }, { value: 'live', label: 'Live chat' }]}
            />
            {previewMode === 'sample' && (
              <Tooltip content={flow ? 'Pause the demo chat' : 'Play a live-feeling demo chat'}>
                <button
                  onClick={() => setFlow((f) => !f)}
                  className="inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-textSecondary hover:text-textPrimary transition-colors"
                >
                  {flow ? <Pause size={13} /> : <Play size={13} />} {flow ? 'Flowing' : 'Flow'}
                </button>
              </Tooltip>
            )}
          </div>
          <div className="flex items-center gap-2">
            <Tooltip content="Preview only. These backdrops just let you check your overlay against different scenes. They don't change your published overlay, that's the Layout background.">
              <span className="text-[11px] text-textMuted cursor-help">Backdrop</span>
            </Tooltip>
            <SegmentedSelect
              value={sceneBg}
              onChange={setSceneBg}
              options={[{ value: 'scene', label: 'Scene' }, { value: 'checker', label: 'Alpha' }, { value: 'dark', label: 'Dark' }, { value: 'light', label: 'Light' }]}
            />
          </div>
        </div>

        {/* The scene fills the pane so the overlay reads as sitting in a real
            layout, not floating in empty space; the canvas is centered and framed
            at true proportion inside it. */}
        <div
          ref={stageWrapRef}
          className="relative w-full flex items-center justify-center rounded-2xl overflow-hidden"
          style={{ height: maxStageH, ...SCENE_STYLES[sceneBg], boxShadow: 'inset 0 0 0 1px rgba(151,177,185,0.16), 0 24px 60px -30px rgba(0,0,0,0.75)' }}
        >
          {sceneBg === 'scene' && (
            <div
              aria-hidden
              className="absolute inset-0 pointer-events-none"
              style={{
                backgroundImage:
                  'linear-gradient(rgba(255,255,255,0.035) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,0.035) 1px, transparent 1px)',
                backgroundSize: '34px 34px',
                maskImage: 'radial-gradient(92% 88% at 50% 45%, #000, transparent)',
                WebkitMaskImage: 'radial-gradient(92% 88% at 50% 45%, #000, transparent)',
              }}
            />
          )}
          <div
            className="relative"
            style={{ width: Math.round(style.width * scale), height: Math.round(style.height * scale), borderRadius: 8, overflow: 'hidden', boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.08)' }}
          >
            <div style={{ position: 'absolute', top: 0, left: 0, width: style.width, height: style.height, transform: `scale(${scale})`, transformOrigin: 'top left' }}>
              {previewMode === 'sample' ? (
                flow ? (
                  <SampleFlowFeed style={style} />
                ) : (
                  <OverlayChat messages={SAMPLE_MESSAGES} style={style} superSample={2} />
                )
              ) : (
                // Kept mounted through all add/remove/swap so the feed diffs
                // connections instead of remounting (which raced the bridge).
                <host.LivePreview sources={sources} style={style} superSample={2} overlayId={profiles[activeIdx]?.id ?? null} />
              )}
            </div>
          </div>
          {/* The canvas size, on the canvas: the number the OBS source has to
              match, shown where the proportions it describes are visible. */}
          <span
            className="pointer-events-none absolute bottom-2 right-2.5 rounded-md px-1.5 py-0.5 text-[10.5px] tabular-nums"
            style={{ background: 'rgba(0,0,0,0.38)', color: 'rgba(255,255,255,0.72)' }}
          >
            {style.width} × {style.height}
          </span>
        </div>

        <p className="px-1 text-[12px] leading-relaxed text-textMuted">
          {previewMode === 'sample' ? 'Sample chat' : 'Live chat'} through the real overlay renderer. Backdrops change only this preview.
        </p>
      </div>
    </div>
  );
};

// Overlays belong to a Twitch account, in the app and on the site alike, so the
// builder asks for sign-in first. Keyed by account: signing in as someone else
// starts a fresh editor that loads that account's overlays.
export const OverlayBuilder = () => {
  const host = useOverlayHost();
  if (!host.accountId) return <SignInGate onSignIn={host.signIn} />;
  return <OverlayEditor key={host.accountId} />;
};

const SignInGate = ({ onSignIn }: { onSignIn: () => void }) => (
  <div className="flex min-h-[320px] items-center justify-center p-6">
    <div className="settings-pinned-card max-w-sm space-y-3 px-5 py-5 text-center">
      <p className="text-[15px] font-semibold text-textPrimary">Sign in to Twitch to build overlays</p>
      <p className="text-[12.5px] leading-relaxed text-textSecondary">
        Your overlays are saved to your Twitch account, so you can edit them in the app or on streamnook.app.
      </p>
      <button
        onClick={onSignIn}
        className="glass-button inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[13px] font-medium text-accent"
      >
        <LogIn size={14} /> Sign in with Twitch
      </button>
    </div>
  </div>
);

export default OverlayBuilder;
