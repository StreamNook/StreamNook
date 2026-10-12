// How the builder's overlay list lines up with the account's overlays.
//
// Published overlays belong to the Twitch account and live on the server, so the
// server copy is the truth for each of them. Drafts (never published) exist only
// in this app or browser and are left exactly as they are. Pure, so both the app
// and the site run the same rule and it can be tested without either.

import { clampOverlayStyle, DEFAULT_OVERLAY_STYLE, OVERLAY_FILE_TEXT_ENTRY_MAX, type OverlayStyle } from '../overlayConfig';
import { PROVIDERS, type ProviderId } from '../../../types/providers';

export interface OverlaySource {
  provider: ProviderId;
  channel: string;
}

export interface OverlayProfile {
  /** Stable client-side identity. Save responses are routed by uid, so a
   *  mid-flight switch can never stamp one overlay's id onto another. Never sent. */
  uid: string;
  name: string;
  /** The published overlay's id (its OBS link), or null for a draft. */
  id: string | null;
  /** The server version this copy was last saved at or loaded from. Sent with
   *  each save so a newer save from elsewhere is never overwritten. */
  version: string | null;
  style: OverlayStyle;
  sources: OverlaySource[];
}

/** One overlay as the account list returns it. */
export interface ServerOverlay {
  id?: unknown;
  channels?: unknown;
  style?: unknown;
  updated_at?: unknown;
}

export const newProfileUid = (): string =>
  typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? crypto.randomUUID()
    : `p-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;

/** A server row as a profile, or null when it is malformed. `uid` lets a local
 *  copy keep its identity when it is refreshed from the server. */
export function profileFromServer(row: ServerOverlay, fallbackName: string, uid = newProfileUid()): OverlayProfile | null {
  if (typeof row.id !== 'string' || !row.id) return null;
  const st = (row.style && typeof row.style === 'object' ? row.style : {}) as Record<string, unknown>;
  const sources = Array.isArray(row.channels)
    ? (row.channels as Array<{ provider?: unknown; channel?: unknown }>)
        .filter((c) => typeof c?.channel === 'string' && !!PROVIDERS[c.provider as ProviderId])
        .map((c) => ({ provider: c.provider as ProviderId, channel: c.channel as string }))
    : [];
  return {
    uid,
    name: typeof st.profileName === 'string' && st.profileName.trim() ? st.profileName : fallbackName,
    id: row.id,
    version: typeof row.updated_at === 'string' ? row.updated_at : null,
    style: clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, ...st } as OverlayStyle),
    sources,
  };
}

export interface Reconciled {
  profiles: OverlayProfile[];
  /** Names of local overlays that are no longer on the account (deleted on
   *  another device, or they belong to a different Twitch account). */
  dropped: string[];
}

/**
 * The builder's list after hearing the account's overlays.
 *
 * - A local overlay the server has takes the server copy (keeping its uid).
 * - A local overlay the server does not have is dropped: it was deleted
 *   elsewhere, or it belongs to another account. Either way its link is not
 *   this account's to edit.
 * - Server overlays not in the list are added after the local ones.
 * - Drafts stay as they are.
 * - The list is never empty: a lone default draft stands in.
 */
export function reconcileProfiles(local: OverlayProfile[], server: ServerOverlay[]): Reconciled {
  // The blank starting overlay a first open creates (one draft, no channels) is
  // not somebody's draft; it only holds the page until the account's overlays
  // arrive, so it gives way to them.
  const blankStart = local.length === 1 && !local[0].id && local[0].sources.length === 0;
  if (blankStart && server.some((r) => typeof r.id === 'string' && r.id)) local = [];

  const byId = new Map<string, ServerOverlay>();
  for (const row of server) if (typeof row.id === 'string' && row.id) byId.set(row.id, row);

  const out: OverlayProfile[] = [];
  const dropped: string[] = [];
  const seen = new Set<string>();
  for (const p of local) {
    if (!p.id) {
      out.push(p);
      continue;
    }
    const row = byId.get(p.id);
    if (!row || seen.has(p.id)) {
      if (!row) dropped.push(p.name);
      continue;
    }
    seen.add(p.id);
    out.push(profileFromServer(row, p.name, p.uid) ?? p);
  }
  let n = out.length;
  for (const row of server) {
    if (typeof row.id !== 'string' || seen.has(row.id)) continue;
    const p = profileFromServer(row, `Overlay ${++n}`);
    if (p) {
      seen.add(p.id as string);
      out.push(p);
    }
  }
  if (out.length === 0) {
    out.push({ uid: newProfileUid(), name: 'Default', id: null, version: null, style: { ...DEFAULT_OVERLAY_STYLE }, sources: [] });
  }
  return { profiles: out, dropped };
}

/**
 * What a save sends for an overlay (its look, its sources and its name, which
 * rides in the sent style as `profileName`) as text that is the same whenever
 * the content is: keys sorted, and the look filled out with defaults and
 * clamped the way loading fills it. The builder auto-saves only when this
 * differs from the copy it last saved or loaded, so opening an overlay, an
 * effect that runs twice, or a default the stored copy lacks never writes over
 * the published link, and a rename alone does save.
 */
export function overlaySaveSnapshot(style: Partial<OverlayStyle>, sources: readonly OverlaySource[], name?: string): string {
  return stableStringify({
    style: clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, ...style } as OverlayStyle),
    sources: sources.map((s) => ({ provider: s.provider, channel: s.channel })),
    name,
  });
}

/** The snapshot of a whole overlay, name included. */
export const profileSaveSnapshot = (p: Pick<OverlayProfile, 'style' | 'sources' | 'name'>): string =>
  overlaySaveSnapshot(p.style, p.sources, p.name);

/** Whether the auto-save sends this overlay: only a published one, and only
 *  when its look, sources or name differ from the copy last saved or loaded
 *  (`saved`, a profileSaveSnapshot). A draft never auto-saves. */
export function overlayNeedsSave(p: Pick<OverlayProfile, 'id' | 'style' | 'sources' | 'name'>, saved: string | undefined): boolean {
  return !!p.id && profileSaveSnapshot(p) !== saved;
}

function stableStringify(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stableStringify).join(',')}]`;
  if (value && typeof value === 'object') {
    const o = value as Record<string, unknown>;
    const keys = Object.keys(o).filter((k) => o[k] !== undefined).sort();
    return `{${keys.map((k) => `${JSON.stringify(k)}:${stableStringify(o[k])}`).join(',')}}`;
  }
  return JSON.stringify(value) ?? 'null';
}

// ── Overlay files ───────────────────────────────────────────────────────────
// An overlay's look, saved to a file one streamer can hand to another. The
// file carries the style and a suggested name only: never the overlay's link
// id, its channels, its per-channel blocked users, or anything about the
// account. Opening one always makes a new draft with its own link to come.
//
// On the desktop app Rust writes and reads these files and checks every field
// (commands/overlay_file.rs). The functions below are the same format for the
// web builder, which has no Rust: browser download and upload.

export const OVERLAY_FILE_FORMAT = 'streamnook-overlay';
export const OVERLAY_FILE_VERSION = 1;
export const OVERLAY_FILE_EXTENSION = 'snoverlay';
export const MAX_OVERLAY_FILE_BYTES = 64 * 1024;
const MAX_OVERLAY_NAME_CHARS = 60;
const DEFAULT_IMPORT_NAME = 'Imported overlay';

/** Style keys that never travel in a file: blockedUsers is keyed by the
 *  sender's own channels, profileName is the published copy of the name. */
const NEVER_EXPORTED = new Set(['blockedUsers', 'profileName']);

/** What opening an overlay file gives the builder. */
export interface ImportedOverlayFile {
  name: string;
  style: Partial<OverlayStyle>;
  /** Style keys in the file that were left out, whole or in part. */
  dropped: string[];
}

// Control characters as the desktop reader sees them (Rust char::is_control:
// C0, DEL and C1).
// eslint-disable-next-line no-control-regex
const CONTROL_CHARS = /[\u0000-\u001f\u007f-\u009f]/g;
// eslint-disable-next-line no-control-regex
const HAS_CONTROL_CHAR = /[\u0000-\u001f\u007f-\u009f]/;
/** Most entries a phrase or command list keeps, as the desktop reader. */
const MAX_LIST_ENTRIES = 100;
// Characters that could end a CSS declaration or open a function.
const UNSAFE_CSS = /[;{}<>\\()]/;
const CSS_FUNCTION_COLOR = /^(rgba?|hsla?)\([\d\s,.%/-]*\)$/i;
const HEX_COLOR = /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;

export const sanitizeOverlayName = (raw: unknown): string => {
  const name = (typeof raw === 'string' ? raw : '').replace(CONTROL_CHARS, '').trim().slice(0, MAX_OVERLAY_NAME_CHARS).trim();
  return name || DEFAULT_IMPORT_NAME;
};

/** A style reduced to the keys OverlayStyle has, each the same JSON type as its
 *  default, strings free of anything that could break out of CSS. The builder
 *  runs clampOverlayStyle on the result for ranges and enums. Hidden phrases and
 *  command filters are read entry by entry exactly as the desktop reader does
 *  (overlay_file.rs), so a file opens the same in the app and on the site. */
export function sanitizeOverlayFileStyle(raw: unknown): { style: Partial<OverlayStyle>; dropped: string[] } {
  const style: Record<string, unknown> = {};
  const dropped: string[] = [];
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return { style, dropped };
  const defaults = DEFAULT_OVERLAY_STYLE as unknown as Record<string, unknown>;
  const kindOf = (v: unknown) => (Array.isArray(v) ? 'array' : v === null ? 'null' : typeof v);
  for (const [key, value] of Object.entries(raw as Record<string, unknown>)) {
    const def = defaults[key];
    const ok =
      !NEVER_EXPORTED.has(key) &&
      Object.prototype.hasOwnProperty.call(defaults, key) &&
      kindOf(value) === kindOf(def) &&
      (typeof value !== 'number' || Number.isFinite(value)) &&
      (typeof value !== 'string' || stringFieldOk(key, value));
    if (!ok) {
      dropped.push(key);
      continue;
    }
    if (key === 'hidePhrases' || key === 'commandFilters') {
      const list = key === 'hidePhrases' ? textEntries(value as unknown[]) : commandEntries(value as unknown[]);
      style[key] = list.entries;
      if (list.lossy) dropped.push(key);
      continue;
    }
    style[key] = value;
  }
  dropped.sort();
  return { style: style as Partial<OverlayStyle>, dropped };
}

/** One phrase or command text: trimmed, no control characters, at most
 *  OVERLAY_FILE_TEXT_ENTRY_MAX characters. '' is an empty entry (skipped
 *  quietly), null one that is left out and reported. */
function listText(v: unknown): string | null {
  if (typeof v !== 'string') return null;
  const s = v.trim();
  if (!s) return '';
  if (HAS_CONTROL_CHAR.test(s) || Array.from(s).length > OVERLAY_FILE_TEXT_ENTRY_MAX) return null;
  return s;
}

function textEntries(raw: unknown[]): { entries: string[]; lossy: boolean } {
  const entries: string[] = [];
  const seen = new Set<string>();
  let lossy = false;
  for (const item of raw) {
    const s = listText(item);
    if (s === null) lossy = true;
    else if (s && !seen.has(s.toLowerCase())) {
      seen.add(s.toLowerCase());
      entries.push(s);
    }
    if (entries.length >= MAX_LIST_ENTRIES) break;
  }
  return { entries, lossy };
}

function commandEntries(raw: unknown[]): { entries: { value: string; mode: 'prefix' | 'exact' }[]; lossy: boolean } {
  const entries: { value: string; mode: 'prefix' | 'exact' }[] = [];
  const seen = new Set<string>();
  let lossy = false;
  for (const item of raw) {
    const rec = item && typeof item === 'object' && !Array.isArray(item) ? (item as Record<string, unknown>) : null;
    const value = rec ? listText(rec.value) : null;
    if (value === null) lossy = true;
    else if (value) {
      const mode = rec?.mode === 'exact' ? 'exact' : 'prefix';
      const k = `${mode}:${value.toLowerCase()}`;
      if (!seen.has(k)) {
        seen.add(k);
        entries.push({ value, mode });
      }
    }
    if (entries.length >= MAX_LIST_ENTRIES) break;
  }
  return { entries, lossy };
}

function stringFieldOk(key: string, value: string): boolean {
  if (value.length > 200 || HAS_CONTROL_CHAR.test(value)) return false;
  if (/color$/i.test(key)) return value === '' || HEX_COLOR.test(value) || CSS_FUNCTION_COLOR.test(value);
  if (key === 'fontFamily') return value.trim() !== '' && !UNSAFE_CSS.test(value);
  return true;
}

/** The file's text for an overlay. */
export function buildOverlayFile(name: string, style: OverlayStyle): string {
  return JSON.stringify(
    {
      format: OVERLAY_FILE_FORMAT,
      version: OVERLAY_FILE_VERSION,
      name: sanitizeOverlayName(name),
      style: sanitizeOverlayFileStyle(style).style,
    },
    null,
    2,
  );
}

/** Read an overlay file's text, or throw an Error whose message is a line for
 *  the builder. */
export function parseOverlayFile(text: string): ImportedOverlayFile {
  if (text.length > MAX_OVERLAY_FILE_BYTES) throw new Error('That file is too large to be an overlay file.');
  let doc: unknown;
  try {
    doc = JSON.parse(text.replace(/^\uFEFF/, ''));
  } catch {
    throw new Error('That file is not a StreamNook overlay file.');
  }
  const rec = doc && typeof doc === 'object' && !Array.isArray(doc) ? (doc as Record<string, unknown>) : null;
  if (!rec || rec.format !== OVERLAY_FILE_FORMAT) throw new Error('That file is not a StreamNook overlay file.');
  const version = rec.version;
  if (typeof version !== 'number' || !Number.isInteger(version) || version < 1) throw new Error('That overlay file has no version.');
  if (version > OVERLAY_FILE_VERSION) throw new Error('That overlay file was made by a newer StreamNook. Update StreamNook to open it.');
  if (!rec.style || typeof rec.style !== 'object' || Array.isArray(rec.style)) throw new Error('That overlay file has no settings in it.');
  const { style, dropped } = sanitizeOverlayFileStyle(rec.style);
  return { name: sanitizeOverlayName(rec.name), style, dropped };
}

/** A file name for an overlay, safe on every desktop OS. */
export function overlayFileName(name: string): string {
  const stem = sanitizeOverlayName(name).replace(/[\\/:*?"<>|]/g, ' ').trim().replace(/\.+$/, '') || 'overlay';
  return `${stem}.${OVERLAY_FILE_EXTENSION}`;
}

/** An imported overlay as a new draft profile: its own uid, no link yet. It
 *  keeps the receiver's sources and blocked users, which are theirs. */
export function profileFromImport(
  file: ImportedOverlayFile,
  name: string,
  current: { sources: OverlaySource[]; style: OverlayStyle },
): OverlayProfile {
  return {
    uid: newProfileUid(),
    name,
    id: null,
    version: null,
    style: clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, ...file.style, blockedUsers: current.style.blockedUsers } as OverlayStyle),
    sources: [...current.sources],
  };
}

// ── Edit history ────────────────────────────────────────────────────────────
// Undo and redo for the builder: one history per overlay (by uid), in memory
// for the session. Each step holds the state an edit replaced. One gesture is
// one step: a change joins the open step while the pointer that started it is
// still down, or when it lands within EDIT_COALESCE_MS of the last change, so a
// slider drag, a run of typing or a colour picker drag undo as one. The builder
// seals the open step on pointer down and up, on blur and on Enter, so a click,
// a toggle or a reset is always a step of its own.

export const EDIT_HISTORY_LIMIT = 100;
export const EDIT_COALESCE_MS = 500;

interface EditStack<T> {
  past: T[];
  future: T[];
  /** The newest step still takes changes. */
  open: boolean;
  lastAt: number;
}

export class EditHistory<T> {
  private readonly stacks = new Map<string, EditStack<T>>();

  constructor(
    private readonly limit = EDIT_HISTORY_LIMIT,
    private readonly coalesceMs = EDIT_COALESCE_MS,
  ) {}

  private stack(key: string): EditStack<T> {
    let s = this.stacks.get(key);
    if (!s) {
      s = { past: [], future: [], open: false, lastAt: 0 };
      this.stacks.set(key, s);
    }
    return s;
  }

  /** An edit on overlay `key` replaced `before`. `holding`: the pointer that
   *  started the gesture is still down. A new step drops what redo held. */
  record(key: string, before: T, now: number, holding = false): void {
    const s = this.stack(key);
    s.future = [];
    if (s.open && (holding || now - s.lastAt < this.coalesceMs)) {
      s.lastAt = now;
      return;
    }
    s.past.push(before);
    if (s.past.length > this.limit) s.past.splice(0, s.past.length - this.limit);
    s.open = true;
    s.lastAt = now;
  }

  /** Close the open step of one overlay, or of every overlay. */
  seal(key?: string): void {
    if (key === undefined) {
      for (const s of this.stacks.values()) s.open = false;
      return;
    }
    const s = this.stacks.get(key);
    if (s) s.open = false;
  }

  /** The state before the latest step, or null when there is none. `current`
   *  becomes the first thing redo gives back. */
  undo(key: string, current: T): T | null {
    const s = this.stacks.get(key);
    if (!s || s.past.length === 0) return null;
    const prev = s.past.pop() as T;
    s.future.push(current);
    s.open = false;
    return prev;
  }

  /** The state the latest undo left, or null when there is none. */
  redo(key: string, current: T): T | null {
    const s = this.stacks.get(key);
    if (!s || s.future.length === 0) return null;
    const next = s.future.pop() as T;
    s.past.push(current);
    if (s.past.length > this.limit) s.past.splice(0, s.past.length - this.limit);
    s.open = false;
    return next;
  }

  canUndo(key: string): boolean {
    return (this.stacks.get(key)?.past.length ?? 0) > 0;
  }

  canRedo(key: string): boolean {
    return (this.stacks.get(key)?.future.length ?? 0) > 0;
  }

  /** Drop an overlay's history (deleted, or replaced by another copy). */
  forget(key: string): void {
    this.stacks.delete(key);
  }
}

export type HistoryShortcutKey = {
  key: string;
  code?: string;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
};

/** Which history action a key press asks for: Ctrl+Z undoes, Ctrl+Shift+Z and
 *  Ctrl+Y redo (Cmd on macOS). Reads the typed letter first, so QWERTZ and
 *  AZERTY get the key labelled Z, and the physical key on other scripts. */
export function historyShortcut(e: HistoryShortcutKey, mac: boolean): 'undo' | 'redo' | null {
  if (e.altKey || !(mac ? e.metaKey : e.ctrlKey) || (mac ? e.ctrlKey : e.metaKey)) return null;
  const letter = /^[a-z]$/i.test(e.key)
    ? e.key.toLowerCase()
    : e.code === 'KeyZ' ? 'z' : e.code === 'KeyY' ? 'y' : '';
  if (letter === 'z') return e.shiftKey ? 'redo' : 'undo';
  if (letter === 'y' && !e.shiftKey) return 'redo';
  return null;
}
