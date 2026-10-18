// 7TV-style emote detail: large multi-size previews, owner, tags, flags, and a
// usage count, plus the actions for this emote (rename, zero-width, remove,
// add to a set, copy link, open on 7tv.app). Opens as a centered modal over the
// workspace from either the set grid or the directory search results.
import { CreatorChip } from '../chat/CreatorChip';
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { motion } from 'framer-motion';
import {
  X, Trash2, Pencil, Check, Layers as LayersIcon, ExternalLink, Copy, Loader2,
  Plus, ChevronDown, Hash,
} from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { Tooltip } from '../ui/Tooltip';
import { SevenTVLogo } from './SevenTVLogo';
import {
  getEmoteDetail, getChannelSets, addEmote, removeEmote, renameEmote, setEmoteZeroWidth, emoteCdnUrl,
  SevenTVSessionExpired, SevenTVGraphQLError,
  type EmoteDetail as EmoteDetailData, type ChannelSet, type EditableChannel,
} from '../../services/seventvEditorService';
import { SevenTvAvatar } from '../SevenTvAvatar';

export interface DetailContext {
  emoteId: string;
  defaultName: string;
  /** Present when opened from a set the channel owns/edits. */
  inSet?: { setId: string; alias: string; zeroWidth: boolean };
}

interface Props {
  ctx: DetailContext;
  canManage: boolean;
  // Single-channel (workspace) mode:
  sets?: ChannelSet[] | null;
  workingSetId?: string | null;
  // Global (chat) mode: a cross-channel add picker instead of one channel's sets.
  channels?: EditableChannel[];
  channelsLoading?: boolean;
  /** Opens the 7TV login flow; shown in the empty state when not connected. */
  onConnect?: () => void;
  onClose: () => void;
  /** Called after any mutation so the grid/drawer can refresh. */
  onChanged?: () => void;
}

const SIZES: { scale: '1x' | '2x' | '3x' | '4x'; px: number }[] = [
  { scale: '1x', px: 32 },
  { scale: '2x', px: 64 },
  { scale: '3x', px: 96 },
  { scale: '4x', px: 128 },
];

export default function EmoteDetail({ ctx, canManage, sets, workingSetId, channels, channelsLoading, onConnect, onClose, onChanged }: Props) {
  const addToast = useAppStore((s) => s.addToast);
  const [data, setData] = useState<EmoteDetailData | null>(null);
  const [loading, setLoading] = useState(true);

  const [alias, setAlias] = useState(ctx.inSet?.alias ?? ctx.defaultName);
  const [zeroWidth, setZeroWidth] = useState(!!ctx.inSet?.zeroWidth);
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(alias);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [busy, setBusy] = useState(false);
  const [removed, setRemoved] = useState(false);

  const manageableSets = (sets ?? []).filter((s) => s.kind === 'NORMAL' || s.kind === 'PERSONAL');
  const [addTargetId, setAddTargetId] = useState<string | null>(workingSetId ?? manageableSets[0]?.id ?? null);
  const [setMenuOpen, setSetMenuOpen] = useState(false);
  const [addedTo, setAddedTo] = useState<Set<string>>(
    new Set(ctx.inSet ? [ctx.inSet.setId] : []),
  );

  const addTarget = manageableSets.find((s) => s.id === addTargetId) || null;

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    getEmoteDetail(ctx.emoteId)
      .then((d) => {
        if (!cancelled) setData(d);
      })
      .catch((e) => {
        if (e instanceof SevenTVSessionExpired) addToast('Your 7TV session expired. Reconnect your 7TV account.', 'error');
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [ctx.emoteId, addToast]);

  const handle = useCallback(
    async (fn: () => Promise<void>, after?: () => void) => {
      setBusy(true);
      try {
        await fn();
        after?.();
        onChanged?.();
      } catch (e) {
        if (e instanceof SevenTVSessionExpired) addToast('Your 7TV session expired. Reconnect your 7TV account.', 'error');
        else if (e instanceof SevenTVGraphQLError) addToast(e.message, 'error');
        else addToast(e instanceof Error ? e.message : 'Action failed', 'error');
      } finally {
        setBusy(false);
      }
    },
    [onChanged, addToast],
  );

  const displayName = ctx.inSet ? alias : (data?.defaultName ?? ctx.defaultName);

  return createPortal(
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/40 backdrop-blur-2xl">
      <div className="absolute inset-0" onClick={onClose} />
      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 12 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ type: 'spring', stiffness: 360, damping: 26 }}
        className="relative z-10 liquid-glass-panel w-[560px] max-w-[92vw] max-h-[86vh] overflow-y-auto custom-scrollbar flex flex-col"
      >
        {/* Header */}
        <div className="flex items-start justify-between px-5 pt-4 shrink-0">
          <div className="min-w-0">
            <div className="text-lg font-semibold text-textPrimary truncate">{displayName}</div>
            {ctx.inSet && data && alias !== data.defaultName && (
              <div className="text-[11px] text-textMuted">original: {data.defaultName}</div>
            )}
            {data?.ownerName && (
              // A valid 7TV user id makes the uploader a chip to their 7TV
              // profile; anything else stays the plain credit line.
              data.ownerId && /^[0-9A-Za-z]{24,26}$/.test(data.ownerId) ? (
                <div className="-ml-0.5 mt-1">
                  <CreatorChip
                    compact
                    creator={{
                      role: 'uploader',
                      name: data.ownerName,
                      avatarUrl: data.ownerAvatarUrl,
                      link: { kind: 'web', url: `https://7tv.app/users/${data.ownerId}`, site: '7TV' },
                    }}
                  />
                </div>
              ) : (
                <div className="flex items-center gap-1.5 mt-1 text-xs text-textSecondary">
                  {data.ownerAvatarUrl && (
                    <SevenTvAvatar
                      cosmeticsId={data.ownerTwitchId}
                      px={16}
                      src={data.ownerAvatarUrl}
                      alt=""
                      className="w-4 h-4 rounded-full object-cover"
                      loading="lazy"
                    />
                  )}
                  <span>by {data.ownerName}</span>
                </div>
              )
            )}
          </div>
          <button onClick={onClose} className="p-1.5 text-textSecondary hover:text-textPrimary hover:bg-glass rounded shrink-0">
            <X size={16} />
          </button>
        </div>

        {/* Preview sizes */}
        <div className="flex items-end justify-center gap-5 px-5 py-5 min-h-[150px]">
          {SIZES.map((s) => (
            <div key={s.scale} className="flex flex-col items-center gap-1.5">
              <div className="flex items-end" style={{ height: 128 }}>
                <PreviewImg id={ctx.emoteId} scale={s.scale} px={s.px} alt={displayName} />
              </div>
              <span className="text-[10px] text-textMuted">{s.px}×{s.px}</span>
            </div>
          ))}
        </div>

        {/* Flags + usage */}
        <div className="flex items-center justify-center gap-2 flex-wrap px-5">
          {loading ? (
            <Loader2 size={14} className="animate-spin text-textSecondary" />
          ) : (
            <>
              {data?.animated && <Chip>Animated</Chip>}
              {(ctx.inSet ? zeroWidth : data?.zeroWidth) && <Chip>Overlay</Chip>}
              {data?.nsfw && <Chip tone="warn">NSFW</Chip>}
              {typeof data?.channelCount === 'number' && (
                <span className="text-[11px] text-textMuted flex items-center gap-1">
                  <Hash size={11} /> {data.channelCount.toLocaleString()} channels
                </span>
              )}
            </>
          )}
        </div>

        {/* Tags */}
        {data?.tags && data.tags.length > 0 && (
          <div className="flex items-center justify-center gap-1.5 flex-wrap px-5 pt-3">
            {data.tags.slice(0, 8).map((t) => (
              <span key={t} className="text-[10px] text-textSecondary bg-glass rounded-full px-2 py-0.5">#{t}</span>
            ))}
          </div>
        )}

        {/* Actions */}
        <div className="px-5 py-4 mt-2 space-y-2">
          {/* In-set actions */}
          {ctx.inSet && canManage && !removed && (
            <div className="flex items-center gap-2">
              {renaming ? (
                <div className="flex items-center gap-1.5 flex-1">
                  <input
                    autoFocus
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        handle(() => renameEmote(ctx.inSet!.setId, ctx.emoteId, draft.trim(), alias), () => { setAlias(draft.trim()); setRenaming(false); });
                      } else if (e.key === 'Escape') { setDraft(alias); setRenaming(false); }
                    }}
                    className="flex-1 px-2 py-1.5 rounded bg-glass text-sm text-textPrimary outline-none focus:ring-1 focus:ring-accent/40"
                  />
                  <button onClick={() => handle(() => renameEmote(ctx.inSet!.setId, ctx.emoteId, draft.trim(), alias), () => { setAlias(draft.trim()); setRenaming(false); })} disabled={busy} className="glass-button px-3 py-1.5 rounded text-sm text-textPrimary">Save</button>
                </div>
              ) : (
                <button onClick={() => { setDraft(alias); setRenaming(true); }} className="glass-button px-3 py-1.5 rounded text-sm text-textPrimary flex items-center gap-1.5">
                  <Pencil size={13} /> Rename
                </button>
              )}
              <Tooltip content={zeroWidth ? 'Unset overlay' : 'Make overlay (zero-width)'} side="top" delay={200}>
                <button
                  onClick={() => handle(() => setEmoteZeroWidth(ctx.inSet!.setId, ctx.emoteId, !zeroWidth, alias), () => setZeroWidth((v) => !v))}
                  disabled={busy}
                  className={`glass-button px-3 py-1.5 rounded text-sm flex items-center gap-1.5 ${zeroWidth ? 'text-accent' : 'text-textPrimary'}`}
                >
                  <LayersIcon size={13} /> Overlay
                </button>
              </Tooltip>
              {confirmRemove ? (
                <div className="flex items-center gap-1 ml-auto">
                  <span className="text-[11px] text-textSecondary">Remove?</span>
                  <button onClick={() => handle(() => removeEmote(ctx.inSet!.setId, ctx.emoteId, alias), () => { setRemoved(true); onClose(); })} disabled={busy} className="px-2 py-1 rounded text-red-400 hover:bg-glass"><Check size={14} /></button>
                  <button onClick={() => setConfirmRemove(false)} className="px-2 py-1 rounded text-textSecondary hover:bg-glass"><X size={14} /></button>
                </div>
              ) : (
                <button onClick={() => setConfirmRemove(true)} className="ml-auto p-1.5 rounded text-textSecondary hover:text-red-400 hover:bg-glass">
                  <Trash2 size={15} />
                </button>
              )}
            </div>
          )}

          {/* Cross-channel add (chat spotlight mode) */}
          {channels && (
            <GlobalAddControl
              emoteId={ctx.emoteId}
              channels={channels}
              channelsLoading={channelsLoading}
              zeroWidthDefault={data?.zeroWidth}
              onConnect={onConnect}
            />
          )}

          {/* Add to a set (single-channel workspace mode) */}
          {!channels && canManage && manageableSets.length > 0 && (
            <div className="flex items-center gap-2">
              <div className="relative flex-1">
                <button
                  onClick={() => setSetMenuOpen((v) => !v)}
                  className="w-full flex items-center justify-between gap-2 glass-button px-3 py-1.5 rounded text-sm text-textPrimary"
                >
                  <span className="truncate">{addTarget ? `Add to: ${addTarget.name}` : 'Choose a set'}</span>
                  <ChevronDown size={14} className="text-textSecondary shrink-0" />
                </button>
                {setMenuOpen && (
                  <>
                    <div className="fixed inset-0 z-10" onClick={() => setSetMenuOpen(false)} />
                    <div className="absolute left-0 bottom-full mb-1 z-20 w-full rounded-lg border border-borderSubtle bg-tertiary shadow-[0_12px_32px_rgba(0,0,0,0.5)] p-1 max-h-56 overflow-y-auto custom-scrollbar">
                      {manageableSets.map((s) => (
                        <button
                          key={s.id}
                          onClick={() => { setAddTargetId(s.id); setSetMenuOpen(false); }}
                          className={`w-full text-left px-2 py-1.5 rounded text-sm flex items-center justify-between ${s.id === addTargetId ? 'bg-glass-active text-textPrimary' : 'text-textSecondary hover:bg-glass'}`}
                        >
                          <span className="truncate">{s.name}</span>
                          {addedTo.has(s.id) && <Check size={13} className="text-emerald-400 shrink-0" />}
                        </button>
                      ))}
                    </div>
                  </>
                )}
              </div>
              {addTarget && addedTo.has(addTarget.id) ? (
                <span className="text-sm text-emerald-400 flex items-center gap-1 px-2"><Check size={15} /> Added</span>
              ) : (
                <button
                  onClick={() =>
                    addTarget &&
                    handle(
                      () => addEmote(addTarget.id, ctx.emoteId, { zeroWidth: data?.zeroWidth || undefined }),
                      () => setAddedTo((s) => new Set(s).add(addTarget.id)),
                    )
                  }
                  disabled={busy || !addTarget}
                  className="glass-button px-3 py-1.5 rounded text-sm text-textPrimary flex items-center gap-1.5 disabled:opacity-60"
                >
                  {busy ? <Loader2 size={14} className="animate-spin" /> : <Plus size={14} />} Add
                </button>
              )}
            </div>
          )}

          {/* Always-available */}
          <div className="flex items-center gap-2 pt-1">
            <button
              onClick={() => { navigator.clipboard?.writeText(`https://7tv.app/emotes/${ctx.emoteId}`); addToast('Emote link copied', 'success'); }}
              className="text-xs text-textSecondary hover:text-textPrimary flex items-center gap-1.5"
            >
              <Copy size={13} /> Copy link
            </button>
            <button
              onClick={() => invoke('open_browser_url', { url: `https://7tv.app/emotes/${ctx.emoteId}` }).catch(() => {})}
              className="text-xs text-textSecondary hover:text-textPrimary flex items-center gap-1.5"
            >
              <ExternalLink size={13} /> Open on 7tv.app
            </button>
          </div>
        </div>
      </motion.div>
    </div>,
    document.body,
  );
}

function Chip({ children, tone }: { children: ReactNode; tone?: 'warn' }) {
  return (
    <span className={`text-[11px] rounded-full px-2 py-0.5 ${tone === 'warn' ? 'text-amber-400 bg-amber-400/10' : 'text-textSecondary bg-glass'}`}>
      {children}
    </span>
  );
}

// A compact opaque dropdown used by the cross-channel add control. Renders its
// menu upward (bottom-full) so it doesn't get clipped at the bottom of the modal.
function Picker({
  label, disabled, children,
}: {
  label: string;
  disabled?: boolean;
  children: (close: () => void) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="relative flex-1 min-w-0">
      <button
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        className="w-full flex items-center justify-between gap-2 glass-button px-3 py-1.5 rounded text-sm text-textPrimary disabled:opacity-50"
      >
        <span className="truncate">{label}</span>
        <ChevronDown size={14} className="text-textSecondary shrink-0" />
      </button>
      {open && !disabled && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} />
          <div className="absolute left-0 bottom-full mb-1 z-20 w-full rounded-lg border border-borderSubtle bg-tertiary shadow-[0_12px_32px_rgba(0,0,0,0.5)] p-1 max-h-56 overflow-y-auto custom-scrollbar">
            {children(() => setOpen(false))}
          </div>
        </>
      )}
    </div>
  );
}

function PickerItem({ active, onClick, children }: { active: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={`w-full text-left px-2 py-1.5 rounded text-sm flex items-center justify-between gap-2 ${
        active ? 'bg-glass-active text-textPrimary' : 'text-textSecondary hover:bg-glass'
      }`}
    >
      {children}
    </button>
  );
}

// Pick any channel you edit, then a set within it, and add the emote there.
// Used when the detail is opened from a chat emote (no fixed channel context).
function GlobalAddControl({
  emoteId, channels, channelsLoading, zeroWidthDefault, onConnect,
}: {
  emoteId: string;
  channels: EditableChannel[];
  channelsLoading?: boolean;
  zeroWidthDefault?: boolean;
  onConnect?: () => void;
}) {
  const addToast = useAppStore((s) => s.addToast);
  const [channelId, setChannelId] = useState<string | null>(channels[0]?.seventvUserId ?? null);

  // Channels may arrive after mount (loaded async by the spotlight host).
  useEffect(() => {
    if (!channelId && channels.length) setChannelId(channels[0].seventvUserId);
  }, [channels, channelId]);
  const [sets, setSets] = useState<ChannelSet[] | null>(null);
  const [setId, setSetId] = useState<string | null>(null);
  const [loadingSets, setLoadingSets] = useState(false);
  const [busy, setBusy] = useState(false);
  const [added, setAdded] = useState<Set<string>>(new Set());

  const channel = channels.find((c) => c.seventvUserId === channelId) || null;

  useEffect(() => {
    if (!channelId) return;
    let cancelled = false;
    setLoadingSets(true);
    setSets(null);
    setSetId(null);
    const isSelf = channels.find((c) => c.seventvUserId === channelId)?.isSelf ?? false;
    getChannelSets(channelId, undefined, isSelf)
      .then((list) => {
        if (cancelled) return;
        const usable = list.filter((s) => s.kind === 'NORMAL' || s.kind === 'PERSONAL');
        setSets(usable);
        const active = usable.find((s) => s.isActive);
        setSetId(active?.id ?? usable[0]?.id ?? null);
      })
      .catch((e) => {
        if (e instanceof SevenTVSessionExpired) addToast('Your 7TV session expired. Reconnect your 7TV account.', 'error');
      })
      .finally(() => {
        if (!cancelled) setLoadingSets(false);
      });
    return () => {
      cancelled = true;
    };
  }, [channelId, channels, addToast]);

  const set = (sets ?? []).find((s) => s.id === setId) || null;
  const key = `${channelId}:${setId}`;
  const isAdded = added.has(key);
  const canAddHere = !!channel?.perms.manageEmotes;

  if (channelsLoading) {
    return (
      <div className="text-[11px] text-textMuted flex items-center gap-2">
        <Loader2 size={13} className="animate-spin" /> Loading your channels…
      </div>
    );
  }
  if (channels.length === 0) {
    return (
      <div className="space-y-2">
        <div className="text-[11px] text-textMuted">
          {onConnect
            ? 'Connect your 7TV account to add this emote to a channel you edit.'
            : 'Connect your 7TV account and be an editor of a channel to add this emote from here.'}
        </div>
        {onConnect && (
          <button
            onClick={onConnect}
            className="glass-button px-3 py-1.5 rounded text-sm text-textPrimary flex items-center gap-2"
          >
            <SevenTVLogo className="h-3.5 w-auto text-[#29b6f6]" /> Connect 7TV
          </button>
        )}
      </div>
    );
  }

  const doAdd = async () => {
    if (!setId) return;
    setBusy(true);
    try {
      await addEmote(setId, emoteId, { zeroWidth: zeroWidthDefault || undefined });
      setAdded((s) => new Set(s).add(key));
      addToast('Emote added', 'success');
    } catch (e) {
      if (e instanceof SevenTVSessionExpired) addToast('Your 7TV session expired. Reconnect your 7TV account.', 'error');
      else if (e instanceof SevenTVGraphQLError) addToast(e.message, 'error');
      else addToast(e instanceof Error ? e.message : 'Could not add emote', 'error');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-1.5">
      <div className="text-[11px] text-textMuted">Add to a channel you edit</div>
      <div className="flex items-center gap-2">
        <Picker label={channel ? channel.displayName : 'Channel'}>
          {(close) =>
            channels.map((c) => (
              <PickerItem
                key={c.seventvUserId}
                active={c.seventvUserId === channelId}
                onClick={() => { setChannelId(c.seventvUserId); close(); }}
              >
                <span className="truncate">{c.displayName}{c.isSelf ? ' (you)' : ''}</span>
              </PickerItem>
            ))
          }
        </Picker>
        <Picker
          label={loadingSets ? 'Loading…' : set ? set.name : 'Set'}
          disabled={loadingSets || !sets || sets.length === 0}
        >
          {(close) =>
            (sets ?? []).map((s) => (
              <PickerItem key={s.id} active={s.id === setId} onClick={() => { setSetId(s.id); close(); }}>
                <span className="truncate">{s.name}</span>
                {added.has(`${channelId}:${s.id}`) && <Check size={13} className="text-emerald-400 shrink-0" />}
              </PickerItem>
            ))
          }
        </Picker>
        {isAdded ? (
          <span className="text-sm text-emerald-400 flex items-center gap-1 px-1 shrink-0"><Check size={15} /> Added</span>
        ) : (
          <button
            onClick={doAdd}
            disabled={busy || !setId || !canAddHere}
            className="glass-button px-3 py-1.5 rounded text-sm text-textPrimary flex items-center gap-1.5 disabled:opacity-60 shrink-0"
          >
            {busy ? <Loader2 size={14} className="animate-spin" /> : <Plus size={14} />} Add
          </button>
        )}
      </div>
      {channel && !canAddHere && (
        <div className="text-[11px] text-amber-400/90">You don't have permission to edit emotes on {channel.displayName}.</div>
      )}
    </div>
  );
}

function PreviewImg({ id, scale, px, alt }: { id: string; scale: '1x' | '2x' | '3x' | '4x'; px: number; alt: string }) {
  const [fallback, setFallback] = useState(false);
  const src = fallback ? `https://cdn.7tv.app/emote/${id}/${scale}.webp` : emoteCdnUrl(id, scale);
  return (
    <img
      src={src}
      alt={alt}
      loading="lazy"
      onError={() => !fallback && setFallback(true)}
      style={{ maxHeight: px, maxWidth: px }}
      className="object-contain"
    />
  );
}
