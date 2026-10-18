// A moderator's warning above the phone's message box: what they said, and
// Acknowledge, which is the one way to chat in this room again. The words are
// Rust's (services/chat_warning.rs, on the room's modes); acknowledging goes
// through Rust too, which rewords the room once Twitch takes it, and that
// unlocks the field.
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ChatCircleText } from 'phosphor-react';
import type { ChatWarningView } from '../../stores/chatConnectionStore';

interface Props {
  warning: ChatWarningView;
  /** The channel's user id. */
  roomId: string;
}

export function WarningBox({ warning, roomId }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const acknowledge = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await invoke('acknowledge_chat_warning', { roomId });
    } catch (err) {
      setError(typeof err === 'string' ? err : 'Could not acknowledge the warning');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mx-2.5 mt-2 rounded-2xl border border-info/25 bg-info/[0.08] p-3" role="alert">
      <div className="flex items-center gap-1.5 text-[14px] font-semibold text-textPrimary">
        <ChatCircleText size={15} weight="fill" className="shrink-0 text-info" />
        {warning.title}
      </div>
      <p className="mt-1 text-[13px] leading-snug text-textSecondary">{warning.detail}</p>
      <button
        type="button"
        onClick={() => void acknowledge()}
        disabled={busy}
        className="mt-2.5 flex h-10 w-full items-center justify-center rounded-xl bg-info text-[15px] font-semibold text-background active:opacity-80 disabled:opacity-60"
      >
        {busy ? 'Acknowledging…' : warning.action}
      </button>
      {error && <p className="mt-1.5 text-[12px] text-error">{error}</p>}
    </div>
  );
}
