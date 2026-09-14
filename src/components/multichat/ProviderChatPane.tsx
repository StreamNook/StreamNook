// ProviderChatPane — read-only chat view for a non-Twitch source (Kick, etc.)
// inside a MultiChat window. Reuses the rich ChatMessageList renderer but skips
// the Twitch-coupled machinery in ChatWidget (Helix polls, viewer counts, send
// input, emote picker, mod menus). Reads the composite "provider:channel" slice
// from the shared chat store. Sending is read-only until per-platform send
// lands; emote/badge images come baked into each message's segments.

import { useCallback, useMemo } from 'react';
import ChatMessageList from '../ChatMessageList';
import { useChannelChat } from '../../stores/chatConnectionStore';
import { makeKey } from '../../utils/providerKey';
import { PROVIDERS, type ProviderId } from '../../types/providers';
import type { BackendChatMessage } from '../../services/twitchChat';
import { Tooltip } from '../ui/Tooltip';

interface ProviderChatPaneProps {
  channel: string; // bare platform slug
  provider: ProviderId;
  channelName?: string;
}

const noop = () => {};

export function ProviderChatPane({ channel, provider, channelName }: ProviderChatPaneProps) {
  const snapshot = useChannelChat(makeKey(provider, channel));
  const meta = PROVIDERS[provider];

  // The store writes a fresh array per change (copy-on-write), so the array
  // is safe to hand down as-is; `renderToken` remains the channel-wide signal.
  const messages = snapshot.messages;

  const getMessageId = useCallback(
    (m: string | BackendChatMessage) => (typeof m === 'string' ? null : m.id),
    [],
  );

  return (
    <div className="flex h-full flex-col bg-background">
      <div className="flex items-center gap-2 px-3 py-1.5 text-xs text-textMuted">
        <span className="font-semibold uppercase tracking-wide" style={{ color: meta?.color }}>
          {meta?.label ?? provider}
        </span>
        <span className="truncate text-textPrimary">{channelName || channel}</span>
        {snapshot.error ? (
          <Tooltip content={snapshot.error}>
            <span className="ml-auto truncate text-red-400">{snapshot.error}</span>
          </Tooltip>
        ) : (
          <span className="ml-auto">{snapshot.isConnected ? 'connected' : 'connecting…'}</span>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-hidden">
        <ChatMessageList
          messages={messages}
          renderToken={snapshot.renderToken}
          isPaused={false}
          onScroll={noop}
          onUsernameClick={noop}
          onReplyClick={noop}
          onEmoteRightClick={noop}
          onUsernameRightClick={noop}
          onBadgeClick={noop}
          highlightedMessageId={null}
          deletedMessageIds={snapshot.deletedMessageIds}
          clearedUserContexts={snapshot.clearedUserContexts}
          emotes={null}
          getMessageId={getMessageId}
        />
      </div>
      <div className="px-3 py-2 text-center text-xs text-textMuted">
        Read-only. Sending to {meta?.label ?? provider} isn&apos;t available yet.
      </div>
    </div>
  );
}

export default ProviderChatPane;
