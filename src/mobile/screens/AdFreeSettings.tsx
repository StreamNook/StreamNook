// Ad-free playback settings.
//
// Its own panel rather than a row inside the shared Player panel: that panel is
// the desktop component, and the desktop app resolves playback through its
// plugin seam, so these two settings mean nothing there.
//
// The relay list is deliberately buried behind a disclosure. The bundled pool
// is what almost everyone should be on, and a hand-typed override that goes
// stale is the most likely way for someone to end up watching ads while the
// feature reports itself as on.
import React, { useCallback, useMemo, useState } from 'react';
import { CaretDown, Path, ShieldCheck } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import type { VideoPlayerSettings } from '../../types';

const Toggle: React.FC<{ on: boolean; onChange: () => void }> = ({ on, onChange }) => (
  <button
    onClick={onChange}
    role="switch"
    aria-checked={on}
    className={`shrink-0 w-11 h-6 rounded-full relative transition-colors ${
      on ? 'bg-accent' : 'bg-surface'
    }`}
  >
    <span
      className="absolute top-0.5 w-5 h-5 rounded-full bg-white transition-[left] duration-200 ease-out"
      style={{ left: on ? 22 : 2 }}
    />
  </button>
);

const SectionLabel: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide mb-1.5 px-1">
    {children}
  </div>
);

/** Plain-language reading of how the stream now playing is being served. */
function describeSource(
  mode: string | undefined,
  entitled: boolean,
  region: string | undefined,
): { label: string; detail: string } {
  switch (mode) {
    case 'turbo':
      return {
        label: 'Turbo',
        detail: 'Your Turbo subscription already serves this stream without ads, so no relay is used.',
      };
    case 'subscribed':
      return {
        label: 'Subscribed',
        detail:
          'Your subscription to this channel already serves it without ads, so no relay is used.',
      };
    case 'proxy':
      return {
        label: region ? `Relay (${region})` : 'Relay',
        detail: 'The playlist is coming through a public relay, with ad segments stripped out.',
      };
    case 'auth-only':
      return {
        label: 'Direct',
        detail:
          'No relay answered, so this is the stream Twitch serves you directly. Ads can appear.',
      };
    default:
      return entitled
        ? { label: 'Ad-free', detail: 'This stream is being served without ads.' }
        : { label: 'Not playing', detail: 'Start a stream to see how it is being served.' };
  }
}

const AdFreeSettings: React.FC = () => {
  const settings = useAppStore((s) => s.settings);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const adSource = useAppStore((s) => s.adSource);
  const [relaysOpen, setRelaysOpen] = useState(false);

  const video: Partial<VideoPlayerSettings> = settings.video_player ?? {};
  const on = video.ad_bypass_enabled !== false;
  const relays = video.ad_bypass_proxies ?? '';

  const patch = useCallback(
    async (changes: Partial<VideoPlayerSettings>) => {
      const current = settings.video_player;
      if (!current) return;
      await updateSettings({ ...settings, video_player: { ...current, ...changes } });
    },
    [settings, updateSettings],
  );

  const source = useMemo(
    () => describeSource(adSource?.mode, !!adSource?.entitled, adSource?.region),
    [adSource],
  );

  return (
    <div className="flex flex-col gap-4">
      <div className="glass-panel p-3.5 flex items-center gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <ShieldCheck size={17} className="text-accent" />
            <span className="text-[14.5px] font-semibold text-textPrimary">Ad-free playback</span>
          </div>
          <div className="text-[12px] text-textMuted leading-snug mt-1">
            {on
              ? 'Routes the playlist through a public relay and removes ad segments before the player sees them.'
              : 'Streams play exactly as Twitch serves them.'}
          </div>
        </div>
        <Toggle on={on} onChange={() => void patch({ ad_bypass_enabled: !on })} />
      </div>

      {on && (
        <>
          <div>
            <SectionLabel>This stream</SectionLabel>
            <div className="glass-panel p-3.5">
              <div className="text-[14.5px] font-semibold text-textPrimary">{source.label}</div>
              <p className="text-[12.5px] text-textSecondary leading-relaxed mt-1">
                {source.detail}
              </p>
            </div>
          </div>

          <div>
            <SectionLabel>Good to know</SectionLabel>
            <div className="glass-panel p-3.5 flex flex-col gap-2.5">
              <p className="text-[12.5px] text-textSecondary leading-relaxed">
                A Turbo subscription or a sub to the channel is already ad-free through Twitch
                itself, and gives you the highest quality tiers. Those streams skip the relay.
              </p>
              <p className="text-[12.5px] text-textSecondary leading-relaxed">
                Low Latency stays off while this is on. Both want to build the playlist the player
                reads, and only one of them can.
              </p>
            </div>
          </div>

          <div>
            <button
              onClick={() => setRelaysOpen((v) => !v)}
              className="w-full flex items-center gap-2 px-1 mb-1.5 text-left"
            >
              <Path size={14} className="text-textMuted" />
              <span className="text-[12px] font-semibold text-textMuted uppercase tracking-wide flex-1">
                Relays
              </span>
              <CaretDown
                size={14}
                className={`text-textMuted transition-transform ${relaysOpen ? 'rotate-180' : ''}`}
              />
            </button>
            {relaysOpen && (
              <div className="glass-panel p-3.5">
                <p className="text-[12.5px] text-textSecondary leading-relaxed mb-2.5">
                  Leave this empty to use the bundled list, which is raced on every stream so the
                  fastest one that answers wins. Enter your own addresses, one per line, only if you
                  run or trust a specific relay.
                </p>
                <p className="text-[12.5px] text-textSecondary leading-relaxed mb-2.5">
                  Your own list is used on its own. If none of them answer, the stream plays
                  directly from Twitch with ads rather than falling back to the bundled relays.
                </p>
                <textarea
                  value={relays}
                  onChange={(e) => void patch({ ad_bypass_proxies: e.target.value })}
                  rows={3}
                  spellCheck={false}
                  autoCapitalize="none"
                  autoCorrect="off"
                  placeholder="https://relay.example.com"
                  className="glass-input w-full rounded-lg px-3 py-2.5 text-[13px] font-mono text-textPrimary placeholder:text-textMuted outline-none resize-none"
                />
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
};

export default AdFreeSettings;
