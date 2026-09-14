import React from 'react';
import { ProviderLogo } from '../ProviderLogo';
import { makeKey } from '../../utils/providerKey';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { Tooltip } from '../ui/Tooltip';

const MultiNookChatSwitcher: React.FC = () => {
  const { slots, activeChatChannelId, setActiveChatChannelId } = usemultiNookStore();

  if (slots.length <= 1) return null; // Only show if multiple streams exist

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
                  px-3 py-1.5 text-xs font-bold tracking-wide transition-all duration-200 flex items-center gap-1.5
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
              </button>
            </Tooltip>
          );
        })}
      </div>
    </div>
  );
};

export default MultiNookChatSwitcher;

