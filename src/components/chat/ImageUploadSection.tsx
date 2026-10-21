// The emote menu's Images section: where pictures go (the host, switchable
// here), "Choose an image", and the uploads, grouped by host with a tab per
// host. A tap puts an upload's link in the message box; an expired one is sent
// again first. Edit deletes uploads (on the host too where it allows that).
// It presents Rust's panel model (image_upload_panel); uploading, the host
// question, deleting and sending again live in services/imageUploadService.

import { useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, ImageUp, Loader2, RefreshCw, Trash2, X } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { IS_MOBILE } from '../../utils/platform';
import type { ImageUploadPanel, UploadedImage, UploadGroup, UploadHost } from '../../types';
import {
  copySrc,
  deleteUpload,
  imageUploadPanel,
  pickImagePath,
  reuploadImage,
  setUploadHost,
  uploadWithConsent,
  type UploadSource,
} from '../../services/imageUploadService';

/** A row's height before it stretches to fill the width. */
const ROW_PX = IS_MOBILE ? 92 : 80;
/** Shapes past these draw whole inside their tile instead of filling it. */
const MIN_ASPECT = 0.8;
const MAX_ASPECT = 4.5;

const headingClass = 'text-[10px] font-bold uppercase tracking-wider text-textSecondary';
/** The round glyph on a tile's corner (delete, sending again). */
const badgeClass =
  'absolute right-1.5 top-1.5 flex h-6 w-6 items-center justify-center rounded-full bg-glass-ink/90 text-white backdrop-blur-sm';

export default function ImageUploadSection({ onInsert }: { onInsert: (text: string) => void }) {
  const [panel, setPanel] = useState<ImageUploadPanel | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<string>('all');
  const [editing, setEditing] = useState(false);
  const [hostsOpen, setHostsOpen] = useState(false);
  const [working, setWorking] = useState<Set<string>>(() => new Set());
  const [broken, setBroken] = useState<Set<string>>(() => new Set());
  const fileRef = useRef<HTMLInputElement>(null);

  const refresh = () => {
    void imageUploadPanel().then(setPanel, () => {});
  };
  useEffect(refresh, []);

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
  const pick = (item: UploadedImage) => {
    if (working.has(item.url)) return;
    if (editing) remove(item);
    else if (item.expired && item.can_reupload) reupload(item, true);
    else onInsert(item.url);
  };
  const pickHost = (h: UploadHost) => {
    setHostsOpen(false);
    if (h.id === panel?.host.id) return;
    void setUploadHost(h.id).then(refresh, () => {});
  };

  const host = panel?.host;
  const groups = panel?.groups ?? [];
  const activeFilter = groups.some((g) => g.host.id === filter) ? filter : 'all';
  const shownGroups = activeFilter === 'all' ? groups : groups.filter((g) => g.host.id === activeFilter);
  const total = groups.reduce((n, g) => n + g.items.length, 0);
  const isEditing = editing && total > 0;
  const deleting = shownGroups.map((g) => g.host.deletes_from_host);
  const editHint = deleting.every(Boolean)
    ? 'Tap a picture to delete it.'
    : deleting.some(Boolean)
      ? 'Tap a picture to delete it. Hosts marked with a bin delete the file; the others keep theirs, so their links still work.'
      : 'Tap a picture to remove it from this list. The host keeps the file, so its link still works.';

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3 scrollbar-thin">
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center gap-1.5 px-1 text-[11px] text-textSecondary">
          <span>Uploads go to</span>
          <button
            type="button"
            onClick={() => setHostsOpen((v) => !v)}
            disabled={!host}
            aria-expanded={hostsOpen}
            className="inline-flex items-center gap-0.5 rounded-md px-1 py-0.5 font-semibold text-textPrimary transition-colors hover:bg-white/[0.06]"
          >
            {host?.label ?? '…'}
            <ChevronDown size={13} className={`transition-transform ${hostsOpen ? 'rotate-180' : ''}`} aria-hidden />
          </button>
        </div>
        {hostsOpen && panel && <HostList hosts={panel.hosts} current={panel.host.id} onPick={pickHost} />}
        <button
          type="button"
          onClick={choose}
          disabled={busy || !panel}
          className="glass-tile flex min-h-[104px] w-full flex-col items-center justify-center gap-1.5 px-4 py-4 text-center transition-colors hover:bg-white/[0.03] disabled:cursor-default disabled:opacity-70"
        >
          {busy ? (
            <Loader2 size={22} className="animate-spin text-textSecondary" aria-hidden />
          ) : (
            <ImageUp size={22} className="text-textSecondary" aria-hidden />
          )}
          <span className="text-[13px] font-medium text-textPrimary">
            {busy ? `Uploading to ${host?.label ?? 'the host'}…` : 'Choose an image'}
          </span>
          {/* Kept (only hidden) while busy, so the lines above do not move. */}
          <span className={`text-[11px] text-textSecondary ${busy ? 'invisible' : ''}`}>JPG, PNG, GIF or WebP</span>
        </button>
        {error && <p className="px-1 text-xs leading-relaxed text-error">{error}</p>}
      </div>

      {total > 0 && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between px-1">
            <h3 className={headingClass}>Uploads</h3>
            <button
              type="button"
              onClick={() => setEditing((v) => !v)}
              className={`rounded-md px-1.5 py-0.5 text-[11px] font-medium transition-colors ${
                isEditing ? 'text-accent' : 'text-textSecondary hover:text-textPrimary'
              }`}
            >
              {isEditing ? 'Done' : 'Edit'}
            </button>
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
                    className={`flex min-w-0 flex-1 items-center justify-center gap-1 rounded-lg px-1.5 py-1.5 text-[11px] transition-all ${
                      activeFilter === t.id ? 'glass-button-active font-bold text-textPrimary' : 'glass-button text-textSecondary hover:text-white'
                    }`}
                  >
                    <span className="truncate">{t.short}</span>
                    <span className="text-[10px] opacity-70 tabular-nums">{t.count}</span>
                  </button>
                ),
              )}
            </div>
          )}
          {isEditing && <p className="px-1 text-[11px] leading-relaxed text-textSecondary">{editHint}</p>}
          {shownGroups.map((g) => (
            <GroupBlock
              key={g.host.id}
              group={g}
              heading={activeFilter === 'all' && groups.length > 1}
              editing={isEditing}
              working={working}
              broken={broken}
              onBroken={(url) => setBroken((prev) => new Set(prev).add(url))}
              onPick={pick}
              onReupload={(item) => reupload(item, false)}
            />
          ))}
        </div>
      )}

      <input ref={fileRef} type="file" accept="image/*" hidden onChange={onFile} />
    </div>
  );
}

function HostList({ hosts, current, onPick }: { hosts: UploadHost[]; current: string; onPick: (h: UploadHost) => void }) {
  return (
    <div className="glass-tile flex flex-col overflow-hidden" role="listbox" aria-label="Where uploads go">
      {hosts.map((h) => {
        const active = h.id === current;
        return (
          <button
            key={h.id}
            type="button"
            role="option"
            aria-selected={active}
            onClick={() => onPick(h)}
            className="flex items-start gap-2 border-b border-borderSubtle px-3 py-2 text-left transition-colors last:border-b-0 hover:bg-white/[0.04]"
          >
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className={`text-[13px] font-medium ${active ? 'text-textPrimary' : 'text-textPrimary/90'}`}>{h.label}</span>
              <span className="text-[11px] leading-snug text-textSecondary">{h.note}</span>
              <span className="text-[10px] text-textMuted">
                up to {h.max_mb} MB{h.expires ? ' · links expire' : ''}
                {h.deletes_from_host ? ' · you can delete uploads' : ''}
              </span>
            </span>
            {active && <Check size={14} className="mt-0.5 flex-shrink-0 text-accent" aria-hidden />}
          </button>
        );
      })}
    </div>
  );
}

function GroupBlock({
  group,
  heading,
  editing,
  working,
  broken,
  onBroken,
  onPick,
  onReupload,
}: {
  group: UploadGroup;
  heading: boolean;
  editing: boolean;
  working: Set<string>;
  broken: Set<string>;
  onBroken: (url: string) => void;
  onPick: (item: UploadedImage) => void;
  onReupload: (item: UploadedImage) => void;
}) {
  // A dead link with no kept copy has nothing to show.
  const items = group.items.filter((i) => !broken.has(i.url) || i.copy_path);
  if (items.length === 0) return null;
  return (
    <div className="flex flex-col gap-1.5">
      {heading && (
        <div className="flex items-baseline justify-between px-1 pt-1">
          <h4 className={headingClass}>{group.host.label}</h4>
          {group.host.expires && <span className="text-[10px] text-textMuted">links expire</span>}
        </div>
      )}
      {/* Justified rows: each tile grows in proportion to its shape, so a row
          shares one height and every picture shows whole. The filler stops
          the last row from stretching. */}
      <div className="flex flex-wrap gap-2">
        {items.map((item) => (
          <Tile
            key={item.url}
            item={item}
            deletes={group.host.deletes_from_host}
            editing={editing}
            working={working.has(item.url)}
            broken={broken.has(item.url)}
            onBroken={() => onBroken(item.url)}
            onPick={() => onPick(item)}
            onReupload={() => onReupload(item)}
          />
        ))}
        <div aria-hidden style={{ flexGrow: 10000, flexBasis: 0, height: 0 }} />
      </div>
    </div>
  );
}

function Tile({
  item,
  deletes,
  editing,
  working,
  broken,
  onBroken,
  onPick,
  onReupload,
}: {
  item: UploadedImage;
  deletes: boolean;
  editing: boolean;
  working: boolean;
  broken: boolean;
  onBroken: () => void;
  onPick: () => void;
  onReupload: () => void;
}) {
  const shape = item.width > 0 && item.height > 0 ? item.width / item.height : 1;
  const aspect = Math.min(MAX_ASPECT, Math.max(MIN_ASPECT, shape));
  const fits = aspect === shape;
  const fromCopy = !!item.copy_path && (item.expired || broken);
  const src = fromCopy && item.copy_path ? copySrc(item.copy_path) : item.url;
  const label = editing
    ? deletes
      ? 'Delete this picture'
      : 'Remove this picture from the list'
    : item.expired
      ? 'Send this picture again and insert its new link'
      : "Insert this image's link";
  return (
    <div className="relative flex min-w-0 flex-col gap-1" style={{ flexGrow: aspect, flexBasis: aspect * ROW_PX }}>
      <button
        type="button"
        onClick={onPick}
        disabled={working}
        aria-label={label}
        title={label}
        className={`glass-tile relative w-full overflow-hidden rounded-lg transition-opacity ${
          editing ? 'hover:opacity-80' : 'hover:opacity-90'
        } ${fits ? '' : 'bg-black/30'}`}
        style={{ aspectRatio: `${aspect}` }}
      >
        <img
          src={src}
          alt=""
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          className={`h-full w-full ${fits ? 'object-cover' : 'object-contain'} ${item.expired ? 'opacity-50' : ''}`}
          onError={fromCopy ? undefined : onBroken}
        />
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
      {/* Sending it again sits on the picture's corner (a glyph, never words
          over the art); how long the link has left sits under it. */}
      {item.can_reupload && !editing && !working && (
        <button
          type="button"
          onClick={onReupload}
          aria-label="Send it again for a fresh link"
          title="Send it again for a fresh link"
          className={`${badgeClass} transition-colors hover:bg-glass-ink/95`}
        >
          <RefreshCw size={12} aria-hidden />
        </button>
      )}
      {item.time_left && (
        <span className={`truncate px-0.5 text-[10px] leading-[14px] ${item.expired ? 'text-warning' : 'text-textMuted'}`}>
          {item.time_left}
        </span>
      )}
    </div>
  );
}
