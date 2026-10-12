// What a /spam or /pyramid run looks like on the page. Rust runs the job
// (commands/chat_burst.rs) and pushes every change as `chat-burst-progress`;
// the desktop shows it above the chat box (ChatBurstStatus), the phone as
// chat notices (watchBurstNotices).

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface BurstStatus {
  channel: string;
  kind: 'spam' | 'pyramid';
  sent: number;
  total: number;
  state: 'running' | 'done' | 'stopped' | 'failed';
  reason?: string;
  delay_ms: number;
}

export const BURST_EVENT = 'chat-burst-progress';

const NOUN: Record<BurstStatus['kind'], string> = { spam: 'Spam', pyramid: 'Pyramid' };

/** "Pyramid 5/9", "Spam done, 20/20", or why a run ended itself. */
export function burstLabel(s: BurstStatus): string {
  const count = `${s.sent}/${s.total}`;
  switch (s.state) {
    case 'running':
      return `${NOUN[s.kind]} ${count}`;
    case 'done':
      return `${NOUN[s.kind]} done, ${count}`;
    case 'stopped':
      return `${NOUN[s.kind]} stopped at ${count}`;
    default:
      return s.reason ?? `${NOUN[s.kind]} stopped at ${count}`;
  }
}

const watching = new Set<string>();

/** Print how the run in `channel` ends (done, stopped or failed) as one chat
 *  notice, for the phone, which has no progress bar above its chat box. */
export async function watchBurstNotices(channel: string, notice: (line: string) => void): Promise<void> {
  const key = channel.toLowerCase();
  if (!key || watching.has(key)) return;
  watching.add(key);
  try {
    const unlisten = await listen<BurstStatus>(BURST_EVENT, ({ payload }) => {
      if (payload.channel !== key || payload.state === 'running' || !watching.has(key)) return;
      watching.delete(key);
      unlisten();
      notice(burstLabel(payload));
    });
    // A run that ended before the listener was in place leaves nothing to wait for.
    const running = await invoke<BurstStatus | null>('get_chat_burst', { channelLogin: key }).catch(() => null);
    if (!running && watching.delete(key)) unlisten();
  } catch {
    watching.delete(key);
  }
}
