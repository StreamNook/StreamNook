// Progress of a /spam or /pyramid run in this channel, above the chat box:
// "Pyramid 5/9" with a Stop button. Rust runs the job (commands/chat_burst.rs)
// and pushes every change as `chat-burst-progress`; this only shows it.
// Mount it keyed by the channel so a channel switch starts clean.

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Repeat, Stop } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { COMPOSER_STRIP, COMPOSER_STRIP_STYLE, GLAZE_PILL } from '../ui/glazeButtons';
import { BURST_EVENT, type BurstStatus } from '../../utils/burstStatus';

// How long a finished run stays on screen.
const LINGER_MS = 4000;

const ChatBurstStatus = ({ channelLogin }: { channelLogin?: string | null }) => {
  const channel = (channelLogin ?? '').toLowerCase();
  const [status, setStatus] = useState<BurstStatus | null>(null);

  useEffect(() => {
    if (!channel) return;
    let live = true;
    let clear: ReturnType<typeof setTimeout> | undefined;
    // A run that started before this chat box mounted.
    invoke<BurstStatus | null>('get_chat_burst', { channelLogin: channel })
      .then((s) => {
        if (live && s) setStatus((cur) => cur ?? s);
      })
      .catch(() => {});
    const unlisten = listen<BurstStatus>(BURST_EVENT, ({ payload }) => {
      if (!live || payload.channel !== channel) return;
      if (clear) clearTimeout(clear);
      setStatus(payload);
      if (payload.state !== 'running') {
        if (payload.state === 'failed') useAppStore.getState().addToast(payload.label, 'warning');
        clear = setTimeout(() => live && setStatus(null), LINGER_MS);
      }
    });
    return () => {
      live = false;
      if (clear) clearTimeout(clear);
      void unlisten.then((f) => f());
    };
  }, [channel]);

  if (!status) return null;
  const running = status.state === 'running';
  return (
    <div className={`${COMPOSER_STRIP} ${running ? '' : 'pr-3'}`} style={COMPOSER_STRIP_STYLE}>
      <Repeat
        size={16}
        weight="bold"
        aria-hidden
        className={`shrink-0 ${status.state === 'failed' ? 'text-warning' : 'text-accent'}`}
      />
      <span className="min-w-0 flex-1 truncate text-[12.5px] leading-[18px] tabular-nums text-textSecondary">{status.label}</span>
      {running && (
        <button
          type="button"
          onClick={() => void invoke('stop_chat_burst', { channelLogin: channel }).catch(() => {})}
          className={`${GLAZE_PILL} inline-flex h-7 shrink-0 items-center gap-1.5 px-3 text-[12px]`}
        >
          <Stop size={11} weight="fill" aria-hidden />
          Stop
        </button>
      )}
    </div>
  );
};

export default ChatBurstStatus;
