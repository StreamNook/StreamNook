import React from 'react';
import { Pin, PinOff } from 'lucide-react';
import { ProviderLogo } from '../ProviderLogo';
import { makeKey } from '../../utils/providerKey';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { Tooltip } from '../ui/Tooltip';

const MultiNookChatSwitcher: React.FC = () => {
  const { slots, activeChatChannelId, setActiveChatChannelId, isChatPinned, toggleChatPinned } = usemultiNookStore();

  if (slots.length <= 1) return null; // Only show if multiple streams exist

  // `no-live-blur` on the chips: the strip's own blur makes it their backdrop
  // root, so theirs could only re-blur the strip, and a backdrop filter nested
  // in another one is what WebView2 paints stale. No transition class either:
  // `.glass-button` and `.glass-input` already list theirs.
  return (
    <div className="flex-shrink-0 flex items-center gap-2 p-2 px-3 overflow-x-auto scrollbar-thin border-b border-borderSubtle bg-glass/30 backdrop-blur-sm shadow-sm" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
      <div className="flex items-center gap-1.5 min-w-max">
        {slots.map((slot) => {
          // The composite slot key is the one identifier that is unambiguous
          // across platforms AND available immediately, so there is no longer an
          // id-then-login fallback to get wrong.
          const key = makeKey(slot.provider ?? 'twitch', slot.channelLogin);
          const isActive = activeChatChannelId === key;

          return (
            <Tooltip key={slot.id} content={`Switch chat to ${slot.channelName || slot.channelLogin}`} side="bottom">
              <button
                onClick={() => setActiveChatChannelId(key)}
                className={`
                  no-live-blur px-3 py-1.5 text-xs font-bold tracking-wide flex items-center gap-1.5
                  ${isActive 
                    ? 'glass-input text-emerald-400 font-extrabold' 
                    : 'glass-button text-textSecondary hover:text-white'}
                `}
                style={{ borderRadius: '8px' }}
              >
                {slot.provider && slot.provider !== 'twitch' && (
                  <ProviderLogo provider={slot.provider} size={11} className="shrink-0" />
                )}
                {slot.channelName || slot.channelLogin}
                {isActive && isChatPinned && <Pin size={11} className="shrink-0" />}
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
          className={`no-live-blur shrink-0 p-1.5 ${
            isChatPinned ? 'glass-input text-emerald-400' : 'glass-button text-textSecondary hover:text-white'
          }`}
          style={{ borderRadius: '8px' }}
        >
          {isChatPinned ? <Pin size={13} /> : <PinOff size={13} />}
        </button>
      </Tooltip>
    </div>
  );
};

export default MultiNookChatSwitcher;

