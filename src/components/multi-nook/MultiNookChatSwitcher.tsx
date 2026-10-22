import React, { useMemo } from 'react';
import { Pin, PinOff } from 'lucide-react';
import { ProviderLogo } from '../ProviderLogo';
import { makeKey } from '../../utils/providerKey';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { Tooltip } from '../ui/Tooltip';
import { ACCENT_BUTTON, ACCENT_FILL } from '../ui/glazeButtons';
import { TabLiveLabel } from '../chat/TabLiveLabel';
import { useTabLive } from '../../hooks/useTabLive';
import { liveWord, type LiveSource } from '../../utils/tabLive';

const NO_SOURCES: LiveSource[] = [];

const MultiNookChatSwitcher: React.FC = () => {
  const slots = usemultiNookStore((s) => s.slots);
  const { activeChatChannelId, setActiveChatChannelId, isChatPinned, toggleChatPinned } = usemultiNookStore();
  // Every tile's channel, docked tiles included: the strip lists them all.
  const sources = useMemo<LiveSource[]>(
    () =>
      slots.length > 1
        ? slots.map((s) => ({
            channel: s.channelLogin,
            channelName: s.channelName,
            provider: s.provider ?? 'twitch',
            channelId: s.channelId ?? null,
          }))
        : NO_SOURCES,
    [slots],
  );
  const live = useTabLive('multinook', sources);

  if (slots.length <= 1) return null; // Only show if multiple streams exist

  // The `--flat` glaze on the chips: the strip's own blur makes it their
  // backdrop root, so theirs could only re-blur the strip, and a backdrop filter
  // nested in another one is what WebView2 paints stale. The chosen chip is the
  // filled accent capsule, like the MultiChat window's tab strip.
  return (
    <div className="flex-shrink-0 flex items-center gap-2 p-2 px-3 overflow-x-auto scrollbar-thin border-b border-borderSubtle bg-glass/30 backdrop-blur-sm shadow-sm" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
      <div className="flex items-center gap-1.5 min-w-max">
        {slots.map((slot) => {
          // The composite slot key is the one identifier that is unambiguous
          // across platforms AND available immediately, so there is no longer an
          // id-then-login fallback to get wrong.
          const key = makeKey(slot.provider ?? 'twitch', slot.channelLogin);
          const isActive = activeChatChannelId === key;
          const name = slot.channelName || slot.channelLogin;
          const state = liveWord(live.byKey.get(key));

          return (
            <Tooltip
              key={slot.id}
              content={state ? `Switch chat to ${name} · ${state}` : `Switch chat to ${name}`}
              side="bottom"
            >
              <button
                onClick={() => setActiveChatChannelId(key)}
                aria-label={state ? `${name}, ${state}` : name}
                className={`
                  px-3 py-1.5 text-xs font-bold tracking-wide flex items-center gap-1.5
                  ${isActive
                    ? `${ACCENT_BUTTON} !text-xs font-extrabold`
                    : 'chrome-glaze chrome-glaze--flat chrome-glaze--control text-textSecondary hover:text-white'}
                `}
                style={isActive ? ACCENT_FILL : undefined}
              >
                {slot.provider && slot.provider !== 'twitch' && (
                  <ProviderLogo provider={slot.provider} size={11} className="shrink-0" />
                )}
                {/* The dot hangs off the name's corner; the pin keeps clear of it. */}
                <TabLiveLabel label={name} isLive={state === 'Live'} />
                {isActive && isChatPinned && <Pin size={11} className="ml-1 shrink-0" />}
              </button>
            </Tooltip>
          );
        })}
      </div>
      <div className="flex-1" />
      <Tooltip
        content={isChatPinned ? 'Unpin chat: follow the focused stream' : 'Pin chat: stay here when focus moves'}
        side="bottom"
      >
        <button
          onClick={toggleChatPinned}
          aria-pressed={isChatPinned}
          aria-label={isChatPinned ? 'Unpin chat' : 'Pin chat'}
          className={`shrink-0 p-1.5 ${
            isChatPinned ? ACCENT_BUTTON : 'chrome-glaze chrome-glaze--flat chrome-glaze--control text-textSecondary hover:text-white'
          }`}
          style={isChatPinned ? ACCENT_FILL : undefined}
        >
          {isChatPinned ? <Pin size={13} /> : <PinOff size={13} />}
        </button>
      </Tooltip>
    </div>
  );
};

export default MultiNookChatSwitcher;
