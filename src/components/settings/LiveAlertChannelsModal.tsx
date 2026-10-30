// Which channels: every followed or favorited channel with its live-alerts
// bell, and Copy from Twitch.
//
// Its own scrim catches a press outside the panel, and it stops there, so the
// Settings dialog under it stays open; Escape closes only this. Rows take
// their state from Rust's list answer (one read, again when the stored mute
// list changes), never one call per row. A press sets the state the row shows
// through the one ordered door (`setLiveAlert`).

import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { Bell, BellOff, X } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { ProviderLogo } from '../ProviderLogo';
import { Tooltip } from '../ui/Tooltip';
import { Logger } from '../../utils/logger';
import { isProviderId } from '../../types/providers';
import { useLiveAlertChannels, type AlertChannelList, type AlertListChannel } from '../../hooks/useLiveAlertChannels';
import {
  copyErrorText,
  copyResultText,
  setLiveAlert,
  sheetIntro,
  undoResultText,
  type CopyAnswer,
} from '../../utils/liveAlerts';

type CopyLine = { text: string; undoId: number | null };

/** Row answers made since the list was read, dropped with the next read. */
interface Overrides {
  forList: AlertChannelList | null;
  on: Record<string, boolean | null>;
}

const rowKey = (c: AlertListChannel) => `${c.provider}:${c.user_id || c.user_login}`;

export function LiveAlertChannelsModal({ onClose }: { onClose: () => void }) {
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
  const { list, failed } = useLiveAlertChannels(true);
  const [query, setQuery] = useState('');
  const [overrides, setOverrides] = useState<Overrides>({ forList: null, on: {} });
  const [busyKeys, setBusyKeys] = useState<ReadonlySet<string>>(new Set());
  const [copying, setCopying] = useState(false);
  const [copyLine, setCopyLine] = useState<CopyLine | null>(null);
  const downOnScrim = useRef(false);

  // Escape closes this list only, never the Settings dialog under it.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopImmediatePropagation();
      onClose();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [onClose]);

  const channels = list?.channels ?? [];
  const rowOn = overrides.forList === list ? overrides.on : {};
  const mixed = new Set(channels.map((c) => c.provider)).size > 1;
  const q = query.trim().toLowerCase();
  const shown = q
    ? channels.filter((c) => c.user_name.toLowerCase().includes(q) || c.user_login.toLowerCase().includes(q))
    : channels;
  const rawLength = useAppStore((s) => s.settings.live_notifications?.muted_live_channels?.length ?? 0);
  // The phones' opening line: undefined while loading, null when the list failed.
  const intro = sheetIntro(list ? list.muted_count : failed ? null : undefined, rawLength);
  // The line under Copy from Twitch: this opening's own result, or (until it
  // has one) the last copy Rust still holds, so a copy that finished after the
  // list closed is never silent. Nothing while a copy runs.
  const lastCopy = list?.last_copy ?? null;
  const shownLine: CopyLine | null = copying
    ? null
    : (copyLine ??
      (lastCopy
        ? { text: copyResultText(lastCopy), undoId: lastCopy.added.length > 0 ? lastCopy.copy_id : null }
        : null));

  const press = (c: AlertListChannel, on: boolean) => {
    const key = rowKey(c);
    const forList = list;
    setBusyKeys((prev) => new Set(prev).add(key));
    setLiveAlert({ provider: c.provider, userId: c.user_id, userLogin: c.user_login }, on)
      .then((answer) => {
        setOverrides((prev) => ({
          forList,
          on: { ...(prev.forList === forList ? prev.on : {}), [key]: answer },
        }));
      })
      .catch((err) => Logger.warn('[LiveAlertChannels] change failed:', err))
      .finally(() =>
        setBusyKeys((prev) => {
          const next = new Set(prev);
          next.delete(key);
          return next;
        }),
      );
  };

  const copyFromTwitch = () => {
    setCopying(true);
    setCopyLine(null);
    invoke<CopyAnswer>('copy_twitch_live_mutes')
      .then((answer) =>
        setCopyLine({ text: copyResultText(answer), undoId: answer.added.length > 0 ? answer.copy_id : null }),
      )
      .catch((err) => setCopyLine({ text: copyErrorText(err), undoId: null }))
      .finally(() => setCopying(false));
  };

  const undoCopy = (copyId: number) => {
    invoke<number>('undo_live_mutes', { copyId })
      .then((count) => setCopyLine({ text: undoResultText(count), undoId: null }))
      .catch((err) => setCopyLine({ text: copyErrorText(err), undoId: null }));
  };

  return createPortal(
    <motion.div
      className="fixed inset-0 z-[calc(var(--sn-z-over-player-lift,0)_+_60)] flex items-center justify-center bg-black/50 p-4"
      initial={motionOff ? false : { opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.16, ease: 'easeOut' }}
      onMouseDown={(e) => {
        downOnScrim.current = e.target === e.currentTarget;
      }}
      onClick={(e) => {
        // Only a press that began and ended on the scrim closes, and it stops
        // here: the Settings dialog under it never hears it.
        e.stopPropagation();
        if (downOnScrim.current && e.target === e.currentTarget) onClose();
        downOnScrim.current = false;
      }}
    >
      <motion.div
        role="dialog"
        aria-modal="true"
        aria-label="Channels to notify me about"
        initial={motionOff ? false : { opacity: 0, scale: 0.98 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.16, ease: 'easeOut' }}
        className="glass-modal flex flex-col overflow-hidden"
        style={{ width: 'min(520px, 94vw)', height: 'min(640px, 88vh)' }}
      >
        <div className="flex h-14 flex-shrink-0 items-center gap-3 border-b border-white/[0.06] px-5">
          <div className="flex-1 text-[15px] font-semibold text-textPrimary">Channels to notify me about</div>
          <Tooltip content="Close" side="bottom">
            <button
              type="button"
              onClick={onClose}
              aria-label="Close"
              className="flex h-8 w-8 items-center justify-center rounded-lg text-textMuted transition-colors hover:bg-white/10 hover:text-textPrimary"
            >
              <X size={16} />
            </button>
          </Tooltip>
        </div>

        <div className="flex-shrink-0 space-y-3 px-5 pt-4 pb-3">
          <p className="text-[12.5px] text-textSecondary">{intro}</p>
          {/* Two lines tall from the start, so a longer result never moves the
              button just pressed or the list below. */}
          <div className="flex min-h-[36px] items-center gap-3">
            <button
              type="button"
              onClick={copyFromTwitch}
              disabled={copying}
              className={`glass-button flex-shrink-0 rounded-lg px-3 py-1.5 text-xs font-semibold text-textPrimary ${
                copying ? 'cursor-wait opacity-60' : ''
              }`}
            >
              Copy from Twitch
            </button>
            <p className="min-w-0 text-[12px] text-textMuted">
              {copying
                ? 'Copying your Twitch bells…'
                : shownLine
                  ? shownLine.text
                  : 'Mutes every channel whose notification bell is off on Twitch.'}
              {shownLine?.undoId != null && (
                <>
                  {' '}
                  <button
                    type="button"
                    onClick={() => undoCopy(shownLine.undoId as number)}
                    className="font-semibold text-accent hover:underline"
                  >
                    Undo
                  </button>
                </>
              )}
            </p>
          </div>
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search your channels"
            aria-label="Search your channels"
            className="glass-input h-8 w-full rounded-lg px-3 text-[13px] text-textPrimary"
          />
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
          {!list && !failed && <p className="px-2 py-3 text-[12.5px] text-textMuted">Loading your follows…</p>}
          {!list && failed && (
            <p className="px-2 py-3 text-[12.5px] text-textMuted">Couldn't load your follows. Close this and open it again.</p>
          )}
          {list && channels.length === 0 && (
            <p className="px-2 py-3 text-[12.5px] text-textMuted">No followed channels found.</p>
          )}
          {list && channels.length > 0 && shown.length === 0 && (
            <p className="px-2 py-3 text-[12.5px] text-textMuted">No channels match that.</p>
          )}
          {shown.map((c) => {
            const key = rowKey(c);
            const on = key in rowOn ? rowOn[key] : c.on;
            const busy = busyKeys.has(key);
            const name = c.user_name || c.user_login;
            return (
              <div key={key} className="flex items-center gap-3 rounded-lg px-2 py-1.5 hover:bg-white/[0.04]">
                {c.profile_image_url ? (
                  <img src={c.profile_image_url} alt="" className="h-8 w-8 flex-shrink-0 rounded-full object-cover" />
                ) : (
                  <div className="h-8 w-8 flex-shrink-0 rounded-full bg-white/[0.06]" />
                )}
                {/* The platform mark after the name, as on the alert rows and tiles. */}
                <span className="flex min-w-0 flex-1 items-center gap-1.5">
                  <span className="min-w-0 truncate text-[13.5px] text-textPrimary">{name}</span>
                  {mixed && isProviderId(c.provider) && <ProviderLogo provider={c.provider} size={12} />}
                </span>
                {on !== null && (
                  <Tooltip content={on ? `Live alerts on for ${name}` : `Live alerts off for ${name}`} side="left">
                    <button
                      type="button"
                      onClick={() => press(c, !on)}
                      disabled={busy}
                      aria-pressed={on}
                      aria-label={on ? `Turn off live alerts for ${name}` : `Turn on live alerts for ${name}`}
                      className={`flex h-8 w-8 items-center justify-center rounded-lg transition-colors hover:bg-white/10 ${
                        busy ? 'cursor-wait opacity-60' : ''
                      }`}
                    >
                      {on ? (
                        <Bell size={16} fill="currentColor" className="text-accent" />
                      ) : (
                        <BellOff size={16} className="text-textMuted" />
                      )}
                    </button>
                  </Tooltip>
                )}
              </div>
            );
          })}
          {list?.truncated && (
            <p className="px-2 py-3 text-[12px] text-textMuted">Only your first 2,000 follows are listed.</p>
          )}
        </div>
      </motion.div>
    </motion.div>,
    document.body,
  );
}
