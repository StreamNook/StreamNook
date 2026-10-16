// What a /spam or /pyramid run looks like on the page. Rust runs the job
// (commands/chat_burst.rs), words every state (`label`) and pushes each change
// as `chat-burst-progress`; the desktop shows it above the chat box
// (ChatBurstStatus). On a phone Rust prints how the run ended in the chat.

export interface BurstStatus {
  channel: string;
  kind: 'spam' | 'pyramid';
  sent: number;
  total: number;
  state: 'running' | 'done' | 'stopped' | 'failed';
  reason?: string;
  delay_ms: number;
  /** "Pyramid 5/9", "Spam done, 20/20", or why a run ended itself. */
  label: string;
}

export const BURST_EVENT = 'chat-burst-progress';
