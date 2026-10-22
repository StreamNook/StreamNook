// The emote menu's Images section: one card to send a picture (and to see or
// switch where it goes), then the uploads, grouped by host with a tab per
// host, shown as a grid of whole pictures or as a list with each one's
// details (host, when it went up, when its link expires, size) and its
// actions. The same section opens full screen as the uploads manager. A tap
// puts an upload's link in the message box; an expired one is sent again
// first. Every picture drawn is the small copy the app keeps on disk, so
// full-size uploads never sit decoded in memory. It presents Rust's panel
// model (image_upload_panel); uploading, the host question, deleting and
// sending again live in services/imageUploadService.

import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import {
  Check,
  ChevronDown,
  Clock3,
  Copy,
  FolderOpen,
  HardDrive,
  ImageUp,
  Infinity as InfinityIcon,
  LayoutGrid,
  List,
  Loader2,
  Maximize2,
  MessageSquarePlus,
  Pencil,
  RefreshCw,
  Trash2,
  X,
} from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { IS_MOBILE } from '../../utils/platform';
import { Tooltip } from '../ui/Tooltip';
import { ACCENT_BUTTON, ACCENT_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import { MobileSheet } from '../../mobile/ui/MobileSheet';
import type { ImageUploadPanel, UploadedImage, UploadGroup, UploadHost } from '../../types';
import {
  copyLink,
  deleteUpload,
  imageUploadPanel,
  localSrc,
  openUploadsFolder,
  pickImagePath,
  reuploadImage,
  setUploadHost,
  setUploadView,
  uploadWithConsent,
  type UploadSource,
} from '../../services/imageUploadService';

type Layout = 'menu' | 'full';

/** A grid row's height before it stretches to fill the width. */
const rowPx = (layout: Layout) => (layout === 'full' ? (IS_MOBILE ? 112 : 150) : IS_MOBILE ? 92 : 80);
/** Shapes past these draw whole inside their tile instead of filling it. */
const MIN_ASPECT = 0.8;
const MAX_ASPECT = 4.5;
/** Labels go stale (time left, "2 hours ago"); the panel is read again this often. */
const REFRESH_MS = 60_000;

// The glaze set: flat blocks inside the menu (no frost of their own),
// white/[0.06] dividers, every button a capsule (glazeButtons).
const headingClass = 'text-[11px] font-semibold uppercase tracking-wider text-white/50';
const BLOCK = 'rounded-xl bg-white/[0.035]';
const DIVIDER = 'border-white/[0.06]';
const ROUND = `flex ${IS_MOBILE ? 'h-8 w-8' : 'h-7 w-7'} flex-shrink-0 items-center justify-center`;
/** The round glyph on a grid tile's corner (delete, sending again). */
const badgeClass =
  'absolute right-1.5 top-1.5 flex h-6 w-6 items-center justify-center rounded-full bg-glass-ink/90 text-white backdrop-blur-sm';
/** A round icon button in a list row. */
const actionClass = `${GLAZE_PILL} ${ROUND}`;

/** The color an upload's time left takes: amber under a day, red once dead. */
const timeTone = (item: UploadedImage) => (item.expired ? 'text-error' : item.urgent ? 'text-warning' : 'text-textMuted');

export default function ImageUploadSection({
  onInsert,
  layout = 'menu',
}: {
  onInsert: (text: string) => void;
  layout?: Layout;
}) {
  const [panel, setPanel] = useState<ImageUploadPanel | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<string>('all');
  const [editing, setEditing] = useState(false);
  const [hostsOpen, setHostsOpen] = useState(false);
  const [managerOpen, setManagerOpen] = useState(false);
  const [working, setWorking] = useState<Set<string>>(() => new Set());
  const [broken, setBroken] = useState<Set<string>>(() => new Set());
  const [copied, setCopied] = useState<string | null>(null);
  const view = useAppStore((s) => s.settings.chat_input?.image_uploader?.view) ?? 'grid';
  const fileRef = useRef<HTMLInputElement>(null);
  const full = layout === 'full';

  const refresh = () => {
    void imageUploadPanel().then(setPanel, () => {});
  };
  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(timer);
  }, []);

  const fail = (message: string) => {
    setError(message);
    if (!IS_MOBILE) useAppStore.getState().addToast(message, 'error');
  };
  const mark = (url: string, on: boolean) =>
    setWorking((prev) => {
      const next = new Set(prev);
      if (on) next.add(url);
      else next.delete(url);
      return next;
    });

  const run = (source: UploadSource) => {
    setError(null);
    void uploadWithConsent(source, () => setBusy(true)).then((out) => {
      setBusy(false);
      if (out.status === 'done') {
        onInsert(out.image.url);
        refresh();
      } else if (out.status === 'failed') {
        fail(out.error);
      }
    });
  };
  const choose = () => {
    if (busy) return;
    if (IS_MOBILE) fileRef.current?.click();
    else void pickImagePath().then((path) => path && run({ path }));
  };
  const onFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = '';
    if (file) run({ file });
  };

  const reupload = (item: UploadedImage, insert: boolean) => {
    setError(null);
    mark(item.url, true);
    void reuploadImage(item.url).then((out) => {
      mark(item.url, false);
      if (out.status === 'done') {
        if (insert) onInsert(out.image.url);
        refresh();
      } else {
        fail(out.error);
      }
    });
  };
  const remove = (item: UploadedImage) => {
    setError(null);
    mark(item.url, true);
    void deleteUpload(item.url).then((out) => {
      mark(item.url, false);
      if (out.status === 'done') refresh();
      else fail(out.error);
    });
  };
  const insert = (item: UploadedImage) => {
    if (working.has(item.url)) return;
    if (item.expired && item.can_reupload) reupload(item, true);
    else onInsert(item.url);
  };
  const copy = (item: UploadedImage) => {
    void copyLink(item.url).then((ok) => {
      if (!ok) return;
      setCopied(item.url);
      setTimeout(() => setCopied((c) => (c === item.url ? null : c)), 1400);
    });
  };
  const pickHost = (h: UploadHost) => {
    setHostsOpen(false);
    if (h.id === panel?.host.id) return;
    void setUploadHost(h.id).then(refresh, () => {});
  };
  const closeManager = () => {
    setManagerOpen(false);
    refresh();
  };

  const host = panel?.host;
  const groups = panel?.groups ?? [];
  const activeFilter = groups.some((g) => g.host.id === filter) ? filter : 'all';
  const shownGroups = activeFilter === 'all' ? groups : groups.filter((g) => g.host.id === activeFilter);
  const total = groups.reduce((n, g) => n + g.items.length, 0);
  const isEditing = editing && total > 0 && view === 'grid';
  const deleting = shownGroups.map((g) => g.host.deletes_from_host);
  const editHint = deleting.every(Boolean)
    ? 'Tap a picture to delete it.'
    : deleting.some(Boolean)
      ? 'Tap a picture to delete it. Hosts marked with a bin delete the file; the others keep theirs, so their links still work.'
      : 'Tap a picture to remove it from this list. The host keeps the file, so its link still works.';

  // Blocks of a scrolling column: shrink-0, or the clipped card collapses.
  const sendCard = (
    <div className={`${BLOCK} flex flex-shrink-0 flex-col overflow-hidden`}>
      <button
        type="button"
        onClick={choose}
        disabled={busy || !panel}
        className="flex items-center gap-3 rounded-t-xl px-3.5 py-3.5 text-left transition-colors hover:bg-white/[0.04] disabled:cursor-default"
      >
        <span className="glaze-inset flex h-11 w-11 flex-shrink-0 items-center justify-center rounded-xl text-white" style={ACCENT_FILL}>
          {busy ? <Loader2 size={20} className="animate-spin" aria-hidden /> : <ImageUp size={20} aria-hidden />}
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-[14px] font-semibold text-textPrimary">
            {busy ? `Uploading to ${host?.label ?? 'the host'}…` : 'Choose an image'}
          </span>
          {/* Kept (only hidden) while busy, so nothing moves. */}
          <span className={`text-[11px] text-textSecondary ${busy ? 'invisible' : ''}`}>
            JPG, PNG, GIF or WebP{host ? `, up to ${host.max_mb} MB` : ''}
          </span>
        </span>
      </button>
      <div className={`flex items-center gap-2.5 border-t ${DIVIDER} px-3.5 py-2.5`}>
        {host ? <HostBadge host={host} size={30} /> : <span className="h-[30px] w-[30px] flex-shrink-0" />}
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="truncate text-[12.5px] font-semibold text-textPrimary">{host?.label ?? '…'}</span>
          {host && <HostChips host={host} />}
        </span>
        <button
          type="button"
          onClick={() => setHostsOpen((v) => !v)}
          disabled={!host}
          aria-expanded={hostsOpen}
          className={`${GLAZE_PILL} inline-flex h-7 flex-shrink-0 items-center gap-1 px-3 text-xs`}
        >
          Change
          <ChevronDown size={12} className={`transition-transform ${hostsOpen ? 'rotate-180' : ''}`} aria-hidden />
        </button>
      </div>
      {hostsOpen && panel && <HostList hosts={panel.hosts} current={panel.host.id} onPick={pickHost} />}
    </div>
  );

  const uploads = total > 0 && (
    <div className="flex flex-shrink-0 flex-col gap-2.5">
      <div className="flex items-center gap-2 px-1">
        <h3 className={headingClass}>Uploads</h3>
        <span className="text-[10px] tabular-nums text-textMuted">{total}</span>
        <span className="flex-1" />
        {!full && (
          <Tooltip content="Open the uploads manager" side="top">
            <button
              type="button"
              onClick={() => setManagerOpen(true)}
              aria-label="Open the uploads manager"
              className={actionClass}
            >
              <Maximize2 size={12} aria-hidden />
            </button>
          </Tooltip>
        )}
        <div className="glaze-inset flex items-center gap-0.5 rounded-full bg-white/[0.08] p-0.5" role="group" aria-label="Show uploads as">
          {(
            [
              ['grid', LayoutGrid, 'Pictures'],
              ['list', List, 'Details'],
            ] as const
          ).map(([id, Icon, label]) => (
            <Tooltip key={id} content={label} side="top">
              <button
                type="button"
                onClick={() => view !== id && void setUploadView(id)}
                aria-pressed={view === id}
                aria-label={label}
                className={`flex h-6 w-7 items-center justify-center rounded-full transition-colors ${
                  view === id ? 'glaze-inset text-white' : 'text-white/55 hover:text-white'
                }`}
                style={view === id ? ACCENT_FILL : undefined}
              >
                <Icon size={13} aria-hidden />
              </button>
            </Tooltip>
          ))}
        </div>
        {view === 'grid' && (
          <button
            type="button"
            onClick={() => setEditing((v) => !v)}
            className={`inline-flex h-7 items-center gap-1 px-3 ${isEditing ? `${ACCENT_BUTTON} !text-xs` : `${GLAZE_PILL} text-xs`}`}
            style={isEditing ? ACCENT_FILL : undefined}
          >
            {isEditing ? <Check size={12} aria-hidden /> : <Pencil size={11} aria-hidden />}
            {isEditing ? 'Done' : 'Edit'}
          </button>
        )}
      </div>
      {groups.length > 1 && (
        <div className="flex gap-1">
          {[{ id: 'all', short: 'All', count: total }, ...groups.map((g) => ({ id: g.host.id, short: g.host.short, count: g.items.length }))].map(
            (t) => (
              <button
                key={t.id}
                type="button"
                onClick={() => setFilter(t.id)}
                aria-pressed={activeFilter === t.id}
                className={`flex h-7 min-w-0 flex-1 items-center justify-center gap-1 px-2 ${
                  activeFilter === t.id ? `${ACCENT_BUTTON} !text-[11.5px]` : `${GLAZE_PILL} text-[11.5px]`
                }`}
                style={activeFilter === t.id ? ACCENT_FILL : undefined}
              >
                <span className="truncate">{t.short}</span>
                <span className="text-[10px] tabular-nums opacity-70">{t.count}</span>
              </button>
            ),
          )}
        </div>
      )}
      {isEditing && <p className="px-1 text-[11px] leading-relaxed text-textSecondary">{editHint}</p>}
      {shownGroups.map((g) => {
        const items = g.items.filter((i) => !broken.has(i.url) || i.copy_path || i.thumb_path);
        if (items.length === 0) return null;
        const onBroken = (url: string) => setBroken((prev) => new Set(prev).add(url));
        return (
          <div key={g.host.id} className="flex flex-col gap-2">
            {activeFilter === 'all' && groups.length > 1 && <GroupHeading group={g} />}
            {view === 'grid' ? (
              // Justified rows: each tile grows in proportion to its shape, so
              // a row shares one height and every picture shows whole. The
              // filler stops the last row from stretching.
              <div className="flex flex-wrap gap-2">
                {items.map((item) => (
                  <Tile
                    key={item.url}
                    item={item}
                    row={rowPx(layout)}
                    deletes={g.host.deletes_from_host}
                    editing={isEditing}
                    working={working.has(item.url)}
                    onBroken={() => onBroken(item.url)}
                    onPick={() => (isEditing ? remove(item) : insert(item))}
                    onReupload={() => reupload(item, false)}
                  />
                ))}
                <div aria-hidden style={{ flexGrow: 10000, flexBasis: 0, height: 0 }} />
              </div>
            ) : (
              <div className={full && !IS_MOBILE ? 'grid grid-cols-2 gap-2' : `${BLOCK} flex flex-col overflow-hidden`}>
                {items.map((item) => (
                  <Row
                    key={item.url}
                    item={item}
                    full={full}
                    hostLabel={g.host.label}
                    deletes={g.host.deletes_from_host}
                    working={working.has(item.url)}
                    copied={copied === item.url}
                    onBroken={() => onBroken(item.url)}
                    onInsert={() => insert(item)}
                    onCopy={() => copy(item)}
                    onReupload={() => reupload(item, false)}
                    onDelete={() => remove(item)}
                  />
                ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );

  return (
    <div className={`flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto scrollbar-thin ${full ? 'p-4' : 'p-3'}`}>
      {full && !IS_MOBILE ? <div className="mx-auto w-full max-w-[560px] flex-shrink-0">{sendCard}</div> : sendCard}
      {error && <p className="-mt-2 flex-shrink-0 px-1 text-xs leading-relaxed text-error">{error}</p>}
      {uploads}
      {full && panel && (
        <div className="mt-auto flex flex-shrink-0 items-center gap-2 px-1 pt-1 text-[11px] text-textMuted">
          <HardDrive size={12} aria-hidden />
          <span>{panel.storage}</span>
          {panel.can_open_folder && (
            <button
              type="button"
              onClick={() => void openUploadsFolder()}
              className={`${GLAZE_PILL} ml-1 inline-flex h-7 items-center gap-1 px-3 text-xs`}
            >
              <FolderOpen size={12} aria-hidden />
              Open folder
            </button>
          )}
        </div>
      )}
      <input ref={fileRef} type="file" accept="image/*" hidden onChange={onFile} />
      {managerOpen && (
        <UploadsManager
          onClose={closeManager}
          onInsert={(text) => {
            onInsert(text);
            closeManager();
          }}
        />
      )}
    </div>
  );
}

/** The section full screen: a dialog on the desktop, the phone's own sheet
 *  there. Rendered from inside the emote menu, so a press in it counts as
 *  inside the menu, which stays open behind it. */
function UploadsManager({ onClose, onInsert }: { onClose: () => void; onInsert: (text: string) => void }) {
  useEffect(() => {
    if (IS_MOBILE) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [onClose]);

  if (IS_MOBILE) {
    return (
      <MobileSheet open onClose={onClose} title="Image uploads" maxHeightFraction={0.94} fill flush>
        <div className="flex h-full min-h-0 flex-col">
          <ImageUploadSection layout="full" onInsert={onInsert} />
        </div>
      </MobileSheet>
    );
  }
  return createPortal(
    <div
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      className="fixed inset-0 z-[calc(var(--sn-z-over-player-lift,0)_+_200)] flex items-center justify-center bg-black/45 p-6"
    >
      <div
        role="dialog"
        aria-modal
        aria-label="Image uploads"
        className="glass-flyout flex h-[min(760px,92vh)] w-[min(1000px,96vw)] flex-col overflow-hidden rounded-2xl !p-0"
      >
        <div className={`flex items-center gap-2 border-b ${DIVIDER} px-4 py-3`}>
          <ImageUp size={16} className="text-accent" aria-hidden />
          <h2 className="text-[14px] font-semibold text-textPrimary">Image uploads</h2>
          <span className="flex-1" />
          <Tooltip content="Close" side="bottom">
            <button
              type="button"
              onClick={onClose}
              aria-label="Close"
              className={actionClass}
            >
              <X size={14} aria-hidden />
            </button>
          </Tooltip>
        </div>
        <ImageUploadSection layout="full" onInsert={onInsert} />
      </div>
    </div>,
    document.body,
  );
}

/** Each host's mark: a tinted tile with its initials (no remote logo, so
 *  showing the list never contacts a host). */
const HOST_MARKS: Record<string, { text: string; color: string }> = {
  nuuls: { text: 'N', color: '#5b8def' },
  kappa: { text: 'K', color: '#9b6bff' },
  imgdrip: { text: 'ID', color: '#22b8cf' },
  catbox: { text: 'CB', color: '#f08c4b' },
  litterbox: { text: 'LB', color: '#e3a92f' },
  uguu: { text: 'U', color: '#f06595' },
};

function HostBadge({ host, size }: { host: UploadHost; size: number }) {
  const mark = HOST_MARKS[host.id] ?? { text: host.short.slice(0, 2).toUpperCase(), color: '#868e96' };
  return (
    <span
      aria-hidden
      className="flex flex-shrink-0 items-center justify-center rounded-lg font-extrabold tracking-tight"
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * (mark.text.length > 1 ? 0.36 : 0.46)),
        color: mark.color,
        background: `color-mix(in srgb, ${mark.color} 16%, transparent)`,
        boxShadow: `inset 0 0 0 1px color-mix(in srgb, ${mark.color} 30%, transparent)`,
      }}
    >
      {mark.text}
    </span>
  );
}

type ChipTone = 'plain' | 'good' | 'warn' | 'bad';
const CHIP_TONE: Record<ChipTone, string> = {
  plain: 'text-textSecondary',
  good: 'text-emerald-300',
  warn: 'text-amber-300',
  bad: 'text-red-300',
};

function Chip({ icon: Icon, tone = 'plain', children }: { icon: typeof Clock3; tone?: ChipTone; children: React.ReactNode }) {
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-full px-2 py-[2px] text-[10px] font-semibold leading-[14px] ${CHIP_TONE[tone]}`}
      style={{ background: 'color-mix(in srgb, currentColor 12%, transparent)' }}
    >
      <Icon size={10} className="flex-shrink-0" aria-hidden />
      {children}
    </span>
  );
}

/** A host's facts as chips: its size limit, how long links last, deleting. */
function HostChips({ host }: { host: UploadHost }) {
  return (
    <span className="flex flex-wrap items-center gap-1">
      <Chip icon={HardDrive}>Up to {host.max_mb} MB</Chip>
      {host.expires ? (
        <Chip icon={Clock3} tone="warn">
          {host.lifetime_hours ? `Links last ${host.lifetime_hours} h` : 'Links expire'}
        </Chip>
      ) : (
        <Chip icon={InfinityIcon} tone="good">
          Kept
        </Chip>
      )}
      {host.deletes_from_host && <Chip icon={Trash2}>Deletable</Chip>}
    </span>
  );
}

function HostList({ hosts, current, onPick }: { hosts: UploadHost[]; current: string; onPick: (h: UploadHost) => void }) {
  return (
    <div className={`flex flex-col gap-1 border-t ${DIVIDER} p-1.5`} role="listbox" aria-label="Where uploads go">
      {hosts.map((h) => {
        const active = h.id === current;
        return (
          <button
            key={h.id}
            type="button"
            role="option"
            aria-selected={active}
            onClick={() => onPick(h)}
            className={`flex items-start gap-3 rounded-lg px-2.5 py-2.5 text-left transition-colors ${
              active ? 'bg-white/[0.08]' : 'hover:bg-white/[0.06]'
            }`}
          >
            <HostBadge host={h} size={34} />
            <span className="flex min-w-0 flex-1 flex-col gap-1">
              <span className="flex items-center gap-1.5">
                <span className="text-[13px] font-semibold text-textPrimary">{h.label}</span>
                {active && <Check size={13} className="text-accent" aria-hidden />}
              </span>
              <span className="text-[11px] leading-snug text-textSecondary">{h.note}</span>
              <HostChips host={h} />
            </span>
          </button>
        );
      })}
    </div>
  );
}

function GroupHeading({ group }: { group: UploadGroup }) {
  return (
    <div className="flex items-center gap-2 px-1 pt-1">
      <HostBadge host={group.host} size={18} />
      <h4 className={headingClass}>{group.host.label}</h4>
      <span className="flex-1" />
      {group.host.expires ? (
        <Chip icon={Clock3} tone="warn">
          {group.host.lifetime_hours ? `Links last ${group.host.lifetime_hours} h` : 'Links expire'}
        </Chip>
      ) : (
        <Chip icon={InfinityIcon} tone="good">
          Kept
        </Chip>
      )}
    </div>
  );
}

/** A picture, whole, from the small copy on disk; else the kept file once
 *  the link is dead, else the link. Falls to the next on a load error. */
function Picture({ item, onBroken, className }: { item: UploadedImage; onBroken: () => void; className: string }) {
  const sources = [
    item.thumb_path && localSrc(item.thumb_path),
    item.expired && item.copy_path && localSrc(item.copy_path),
    !item.expired && item.url,
    item.copy_path && localSrc(item.copy_path),
  ].filter((s): s is string => !!s);
  const [at, setAt] = useState(0);
  const src = sources[Math.min(at, sources.length - 1)];
  return (
    <img
      src={src}
      alt=""
      loading="lazy"
      decoding="async"
      referrerPolicy="no-referrer"
      className={`${className} ${item.expired ? 'opacity-50' : ''}`}
      onError={() => (at + 1 < sources.length ? setAt(at + 1) : onBroken())}
    />
  );
}

function Tile({
  item,
  row,
  deletes,
  editing,
  working,
  onBroken,
  onPick,
  onReupload,
}: {
  item: UploadedImage;
  row: number;
  deletes: boolean;
  editing: boolean;
  working: boolean;
  onBroken: () => void;
  onPick: () => void;
  onReupload: () => void;
}) {
  const shape = item.width > 0 && item.height > 0 ? item.width / item.height : 1;
  const aspect = Math.min(MAX_ASPECT, Math.max(MIN_ASPECT, shape));
  const fits = aspect === shape;
  const label = editing
    ? deletes
      ? 'Delete this picture'
      : 'Remove this picture from the list'
    : item.expired
      ? 'Send this picture again and insert its new link'
      : "Insert this picture's link";
  const tipLines = [item.details, `Uploaded ${item.uploaded_at}`, item.expiry].filter(Boolean);
  // One line each, as the native title drew them; the screen reader gets them
  // as the button's description.
  const tip = (
    <span className="block space-y-0.5 text-left">
      {tipLines.map((l) => (
        <span key={l} className="block">
          {l}
        </span>
      ))}
    </span>
  );
  return (
    <div className="relative flex min-w-0 flex-col gap-1" style={{ flexGrow: aspect, flexBasis: aspect * row }}>
      <Tooltip content={tip}>
        <button
          type="button"
          onClick={onPick}
          disabled={working}
          aria-label={label}
          aria-description={tipLines.join('. ')}
          className={`relative w-full overflow-hidden rounded-xl bg-white/[0.04] transition-opacity ${
            editing ? 'hover:opacity-80' : 'hover:opacity-90'
          } ${fits ? '' : 'bg-black/30'}`}
          style={{ aspectRatio: `${aspect}` }}
        >
          <Picture item={item} onBroken={onBroken} className={`h-full w-full ${fits ? 'object-cover' : 'object-contain'}`} />
          {(working || editing) && (
            <span className={badgeClass}>
              {working ? (
                <Loader2 size={13} className="animate-spin" aria-hidden />
              ) : deletes ? (
                <Trash2 size={13} aria-hidden />
              ) : (
                <X size={13} aria-hidden />
              )}
            </span>
          )}
        </button>
      </Tooltip>
      {/* Sending it again sits on the picture's corner (a glyph, never words
          over the art); its age or time left sits under it. */}
      {item.can_reupload && (item.expires_ms || item.expired) && !editing && !working && (
        <Tooltip content="Send it again for a fresh link">
          <button
            type="button"
            onClick={onReupload}
            aria-label="Send it again for a fresh link"
            className={`${badgeClass} transition-colors hover:bg-glass-ink/95`}
          >
            <RefreshCw size={12} aria-hidden />
          </button>
        </Tooltip>
      )}
      <span className={`flex items-center gap-1 truncate px-0.5 text-[10px] leading-[14px] ${item.time_left ? timeTone(item) : 'text-textMuted'}`}>
        {item.time_left && <Clock3 size={10} className="flex-shrink-0" aria-hidden />}
        <span className="truncate">{item.time_left ?? item.uploaded}</span>
      </span>
    </div>
  );
}

function Row({
  item,
  full,
  hostLabel,
  deletes,
  working,
  copied,
  onBroken,
  onInsert,
  onCopy,
  onReupload,
  onDelete,
}: {
  item: UploadedImage;
  full: boolean;
  hostLabel: string;
  deletes: boolean;
  working: boolean;
  copied: boolean;
  onBroken: () => void;
  onInsert: () => void;
  onCopy: () => void;
  onReupload: () => void;
  onDelete: () => void;
}) {
  const thumb = full ? (IS_MOBILE ? 88 : 96) : IS_MOBILE ? 76 : 68;
  return (
    <div
      className={`flex gap-3 px-3 py-3 ${
        full && !IS_MOBILE ? BLOCK : `border-b ${DIVIDER} last:border-b-0`
      }`}
    >
      <button
        type="button"
        onClick={onInsert}
        disabled={working}
        aria-label={item.expired ? 'Send this picture again and insert its new link' : "Insert this picture's link"}
        className="flex flex-shrink-0 items-center justify-center overflow-hidden rounded-lg bg-black/30 transition-opacity hover:opacity-90"
        style={{ width: thumb, height: thumb }}
      >
        <Picture item={item} onBroken={onBroken} className="h-full w-full object-contain" />
      </button>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-baseline gap-1.5">
          <span className="truncate text-[12.5px] font-semibold text-textPrimary">{hostLabel}</span>
          <Tooltip content={item.uploaded_at} describe>
            <span className="flex-shrink-0 text-[11px] text-textMuted">
              {item.uploaded}
            </span>
          </Tooltip>
        </div>
        <span className="flex flex-wrap">
          {item.expires_ms ? (
            <Chip icon={Clock3} tone={item.expired ? 'bad' : item.urgent ? 'warn' : 'plain'}>
              {item.expiry}
            </Chip>
          ) : (
            <Chip icon={InfinityIcon} tone={item.expiry === 'Kept indefinitely' ? 'good' : 'warn'}>
              {item.expiry}
            </Chip>
          )}
        </span>
        {item.details && <span className="text-[11px] text-textMuted">{item.details}</span>}
        <div className="mt-1 flex items-center gap-1.5">
          <Tooltip content={item.expired ? 'Send again and insert' : 'Insert link'} side="top">
            <button type="button" onClick={onInsert} disabled={working} aria-label="Insert link" className={`${ACCENT_BUTTON} ${ROUND}`} style={ACCENT_FILL}>
              <MessageSquarePlus size={14} aria-hidden />
            </button>
          </Tooltip>
          <Tooltip content={copied ? 'Copied' : 'Copy link'} side="top">
            <button type="button" onClick={onCopy} disabled={working || item.expired} aria-label="Copy link" className={actionClass}>
              {copied ? <Check size={14} className="text-accent" aria-hidden /> : <Copy size={13} aria-hidden />}
            </button>
          </Tooltip>
          {item.can_reupload && (item.expires_ms || item.expired) && (
            <Tooltip content="Send again for a fresh link" side="top">
              <button type="button" onClick={onReupload} disabled={working} aria-label="Send again for a fresh link" className={actionClass}>
                {working ? <Loader2 size={13} className="animate-spin" aria-hidden /> : <RefreshCw size={13} aria-hidden />}
              </button>
            </Tooltip>
          )}
          <span className="flex-1" />
          <Tooltip content={deletes ? `Delete from ${hostLabel}` : 'Remove from this list (the host keeps the file)'} side="top">
            <button
              type="button"
              onClick={onDelete}
              disabled={working}
              aria-label={deletes ? 'Delete' : 'Remove from list'}
              className={`${actionClass} hover:!text-error`}
            >
              {deletes ? <Trash2 size={13} aria-hidden /> : <X size={14} aria-hidden />}
            </button>
          </Tooltip>
        </div>
      </div>
    </div>
  );
}
