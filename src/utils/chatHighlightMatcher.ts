// Highlight rules are evaluated in Rust (src-tauri/src/services/chat_rules.rs)
// and stamped onto each message's metadata; the chat row reads the stamp. The
// JS matcher that lived here is retired. Only the presentation shape remains.

import type { SoundRef } from './notificationSound';

export interface HighlightMatch {
  phrase_id: string;
  color: string;
  sound_id: SoundRef | null;
  cooldown_ms: number;
}
