// Per-channel live-alert control over the full follow list, with Copy from
// Twitch at the top.
//
// A sheet rather than a settings sub-page because `settingsView` holds one id
// with no stack, so a nested panel's back press would land on the You tab
// instead of returning here. Sheets already register with the nav store's
// stack and pop correctly on Android back.
//
// Fed by get_followed_channels_for_alerts: Rust pages through every channel
// you follow, live or not, stops at a fixed number of pages, sorts by name and
// says per channel whether its alerts are on. Rows show Rust's answer; a store
// change (a press here, the player's bell, a Copy) re-reads it, and a newer
// read always wins over an older one.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Bell, BellSlash, MagnifyingGlass } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import {
  alertChannels,
  copyTwitchBells,
  setLiveAlert,
  undoTwitchBells,
  type AlertChannel,
} from '../notifyChannels';
import { copyErrorText, copyResultText, sheetIntro, undoResultText } from '../liveAlertsText';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import { sevenTvIdForStream } from '../../services/sevenTvPictures';

/** The line under Copy from Twitch: what the last Copy or Undo did. Undo is
 *  offered only while `copyId` is set and the sheet stays open. */
interface ResultLine {
  text: string;
  copyId: number | null;
}

export const NotifiedChannelsSheet: React.FC<{ open: boolean; onClose: () => void }> = ({
  open,
  onClose,
}) => {
  const [channels, setChannels] = useState<AlertChannel[]>([]);
  const [loading, setLoading] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const [mutedCount, setMutedCount] = useState<number | null | undefined>(undefined);
  const [query, setQuery] = useState('');
  const [copying, setCopying] = useState(false);
  const [undoing, setUndoing] = useState(false);
  const [line, setLine] = useState<ResultLine | null>(null);

  // A new opening starts with no line of its own; the first list read brings
  // back the last Copy (and its Undo) when Rust still holds it, so a copy that
  // finished after the sheet closed is never silent (PLAN 4.12).
  const [lastOpen, setLastOpen] = useState(open);
  if (open !== lastOpen) {
    setLastOpen(open);
    if (open) setLine(null);
  }

  // The stored list: its changes ask Rust again, and its length is the
  // fallback count while Rust's answer is on its way.
  const muted = useAppStore((s) => s.settings.live_notifications?.muted_live_channels);
  const rawLength = muted?.length ?? 0;

  // Bumped by every read; an answer from an older read is dropped.
  const readSeq = useRef(0);

  useEffect(() => {
    if (!open) return;
    const mine = ++readSeq.current;

    const load = async () => {
      setLoading(true);
      try {
        const result = await alertChannels();
        if (mine === readSeq.current) {
          setChannels(result.channels);
          setTruncated(result.truncated);
          setMutedCount(result.muted_count);
          const last = result.last_copy;
          if (last) {
            setLine(
              (prev) =>
                prev ?? {
                  text: copyResultText(last),
                  copyId: last.added.length > 0 ? last.copy_id : null,
                },
            );
          }
        }
      } catch (err) {
        Logger.warn('[NotifiedChannels] follow list failed:', err);
      } finally {
        if (mine === readSeq.current) setLoading(false);
      }
    };

    void load();
  }, [open, muted]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return channels;
    return channels.filter(
      (c) => c.user_name.toLowerCase().includes(q) || c.user_login.toLowerCase().includes(q),
    );
  }, [channels, query]);

  const onToggle = useCallback((c: AlertChannel) => {
    const shownOn = c.on !== false;
    void setLiveAlert({ provider: 'twitch', userId: c.user_id, userLogin: c.user_login }, !shownOn).then((on) => {
      if (typeof on !== 'boolean') return;
      setChannels((prev) => prev.map((x) => (x.user_id === c.user_id ? { ...x, on } : x)));
    });
  }, []);

  const onCopy = useCallback(async () => {
    setCopying(true);
    const result = await copyTwitchBells();
    setCopying(false);
    if (result.ok) {
      setLine({
        text: copyResultText(result.answer),
        copyId: result.answer.added.length > 0 ? result.answer.copy_id : null,
      });
    } else {
      setLine({ text: copyErrorText(result.code), copyId: null });
    }
  }, []);

  const onUndo = useCallback(async (copyId: number) => {
    setUndoing(true);
    const result = await undoTwitchBells(copyId);
    setUndoing(false);
    setLine({ text: result.ok ? undoResultText(result.removed) : copyErrorText(result.code), copyId: null });
  }, []);

  return (
    <MobileSheet open={open} onClose={onClose} title="Channels to notify me about">
      <div className="flex flex-col gap-3 px-3.5 pb-3.5">
        <p className="text-[12.5px] text-textSecondary leading-relaxed">{sheetIntro(mutedCount, rawLength)}</p>

        {/* One tap brings Twitch's own choices over: every channel whose bell is
            off on Twitch joins the muted list. Its result stays on one line
            right under the button, with Undo while the sheet is open. */}
        <div className="flex flex-col gap-1.5">
          <button
            onClick={() => void onCopy()}
            disabled={copying || undoing}
            className="glass-button w-full py-2.5 text-[14px] font-medium text-textPrimary disabled:opacity-60"
          >
            {copying ? 'Copying your Twitch bells…' : 'Copy from Twitch'}
          </button>
          {/* Always drawn, two lines tall, so a result never moves the list
              under the finger that just pressed Copy. */}
          <p className="min-h-[35px] text-[12.5px] text-textSecondary leading-snug" aria-live="polite">
            {line ? line.text : 'Mutes every channel whose notification bell is off on Twitch.'}
            {line && line.copyId !== null && (
                <>
                  {' '}
                  <button
                    onClick={() => {
                      if (line.copyId !== null) void onUndo(line.copyId);
                    }}
                    disabled={undoing}
                    className="font-semibold text-accent disabled:opacity-60"
                  >
                    Undo
                  </button>
                </>
              )}
          </p>
        </div>

        <div className="relative">
          <MagnifyingGlass
            size={16}
            className="absolute left-3 top-1/2 -translate-y-1/2 text-textMuted z-10"
          />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search channels"
            className="glass-input w-full pl-9 pr-3 py-2.5 text-[14px] text-textPrimary"
          />
        </div>

        {loading && channels.length === 0 ? (
          <div className="text-[13px] text-textMuted py-6 text-center">Loading your follows…</div>
        ) : filtered.length === 0 ? (
          <div className="text-[13px] text-textMuted py-6 text-center">
            {channels.length === 0 ? 'No followed channels found.' : 'No channels match that.'}
          </div>
        ) : (
          <div className="glass-panel divide-y divide-borderSubtle">
            {filtered.map((c) => {
              const off = c.on === false;
              return (
                <div key={c.user_id} className="flex items-center gap-3 p-2.5">
                  {c.profile_image_url ? (
                    <SevenTvAvatar
                      cosmeticsId={sevenTvIdForStream(c)}
                      px={32}
                      src={c.profile_image_url}
                      alt=""
                      className="w-8 h-8 rounded-full shrink-0"
                      loading="lazy"
                    />
                  ) : (
                    <div className="w-8 h-8 rounded-full bg-surface shrink-0" />
                  )}
                  <div className="flex-1 min-w-0">
                    <div className="text-[14px] text-textPrimary truncate">{c.user_name}</div>
                  </div>
                  <button
                    onClick={() => onToggle(c)}
                    aria-label={off ? `Turn on alerts for ${c.user_name}` : `Turn off alerts for ${c.user_name}`}
                    aria-pressed={!off}
                    className={`sn-touch shrink-0 flex items-center justify-center transition-colors ${
                      off ? 'text-textMuted' : 'text-accent'
                    }`}
                  >
                    {off ? <BellSlash size={19} /> : <Bell size={19} weight="fill" />}
                  </button>
                </div>
              );
            })}
          </div>
        )}

        {truncated && (
          <p className="text-[12px] text-textMuted leading-snug">
            Showing the first {channels.length} channels you follow. Search covers only these.
          </p>
        )}
      </div>
    </MobileSheet>
  );
};

export default NotifiedChannelsSheet;
