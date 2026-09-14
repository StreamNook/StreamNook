// Per-channel live-alert control over the full follow list.
//
// A sheet rather than a settings sub-page because `settingsView` holds one id
// with no stack, so a nested panel's back press would land on the You tab
// instead of returning here. Sheets already register with the nav store's
// stack and pop correctly on Android back.
//
// Fed by get_all_followed_channels, which has been registered and ungated the
// whole time with nothing in the frontend calling it. Note that it returns
// every channel you follow, live or not, which is exactly what this needs and
// is a different endpoint from the followed-STREAMS one the alerts run on.
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Bell, BellSlash, MagnifyingGlass } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { isChannelMuted, toggleChannelMuted } from '../notifyChannels';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import type { TwitchStream } from '../../types';

const PAGE = 100;
// Enough for anyone's follow list without letting a pathological account spin
// through requests forever. Whatever is dropped is stated in the UI rather than
// silently truncated.
const MAX_PAGES = 20;

export const NotifiedChannelsSheet: React.FC<{ open: boolean; onClose: () => void }> = ({
  open,
  onClose,
}) => {
  const [channels, setChannels] = useState<TwitchStream[]>([]);
  const [loading, setLoading] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const [query, setQuery] = useState('');

  // Subscribing to the settings object keeps every row's bell in step with the
  // store, so a toggle does not need local mirror state that could drift.
  const muted = useAppStore((s) => s.settings.live_notifications?.muted_live_channels);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;

    const load = async () => {
      setLoading(true);
      const all: TwitchStream[] = [];
      let cursor: string | undefined;
      let pages = 0;
      try {
        for (;;) {
          const [batch, next] = await invoke<[TwitchStream[], string | null]>(
            'get_all_followed_channels',
            { limit: PAGE, cursor: cursor ?? null },
          );
          if (cancelled) return;
          all.push(...batch);
          pages += 1;
          if (!next || batch.length === 0) break;
          if (pages >= MAX_PAGES) {
            if (!cancelled) setTruncated(true);
            break;
          }
          cursor = next;
        }
        if (!cancelled) {
          all.sort((a, b) => a.user_name.localeCompare(b.user_name));
          setChannels(all);
        }
      } catch (err) {
        Logger.warn('[NotifiedChannels] follow list failed:', err);
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    void load();
    return () => {
      cancelled = true;
    };
  }, [open]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return channels;
    return channels.filter(
      (c) => c.user_name.toLowerCase().includes(q) || c.user_login.toLowerCase().includes(q),
    );
  }, [channels, query]);

  const mutedCount = muted?.length ?? 0;

  const onToggle = useCallback((login: string) => {
    void toggleChannelMuted(login);
  }, []);

  return (
    <MobileSheet open={open} onClose={onClose} title="Channels to notify me about">
      <div className="flex flex-col gap-3 px-3.5 pb-3.5">
        <p className="text-[12.5px] text-textSecondary leading-relaxed">
          {mutedCount > 0
            ? `Alerting for every channel you follow except ${mutedCount}.`
            : 'Alerting for every channel you follow. Turn off any you would rather not hear about.'}
        </p>

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
              const off = isChannelMuted(c.user_login);
              return (
                <div key={c.user_id} className="flex items-center gap-3 p-2.5">
                  {c.profile_image_url ? (
                    <img
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
                    onClick={() => onToggle(c.user_login)}
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
            Showing the first {MAX_PAGES * PAGE} channels you follow. Search covers only these.
          </p>
        )}
      </div>
    </MobileSheet>
  );
};

export default NotifiedChannelsSheet;
