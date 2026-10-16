// Progress of a /spam or /pyramid run in this channel, above the chat box:
// "Pyramid 5/9" with a Stop button. Rust runs the job (commands/chat_burst.rs)
// and pushes every change as `chat-burst-progress`; this only shows it.
// Mount it keyed by the channel so a channel switch starts clean.

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Repeat, Square } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
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
    <div className="mb-2 flex items-center gap-2 px-3 py-2 bg-glass rounded-lg border border-borderSubtle">
      <Repeat size={14} className={`flex-shrink-0 ${status.state === 'failed' ? 'text-warning' : 'text-accent'}`} />
      <span className="text-xs text-textSecondary flex-1 min-w-0 truncate tabular-nums">{status.label}</span>
      {running && (
        <button
          type="button"
          onClick={() => void invoke('stop_chat_burst', { channelLogin: channel }).catch(() => {})}
          className="glass-button inline-flex flex-shrink-0 items-center gap-1 rounded-md px-2 py-0.5 text-[11px] text-textPrimary"
        >
          <Square size={10} />
          Stop
        </button>
      )}
    </div>
  );
};

export default ChatBurstStatus;
