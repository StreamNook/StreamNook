// A message AutoMod held, drawn in chat where it arrived: why it is held (in
// Rust's words), the author and text, and Allow and Deny under it, words not
// icons and big enough to hit in a moving chat. Once a moderator decides it,
// the row folds to one quiet line saying what happened and who did it.
import { memo, useState } from 'react';
import { Flag } from 'phosphor-react';
import { useAutomodStore, type HeldRowData } from '../../stores/automodStore';

interface HeldMessageRowProps {
  row: HeldRowData;
}

function HeldMessageRow({ row }: HeldMessageRowProps) {
  const [busy, setBusy] = useState<'allow' | 'deny' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { held } = row;
  const name = held.user_name || held.user_login;

  if (row.line) {
    return (
      <div className="flex items-start gap-1.5 px-3 py-1 text-[12px] leading-snug text-textSecondary">
        <Flag size={12} weight="fill" className="mt-[2px] flex-shrink-0 text-textMuted" />
        <span>
          {row.line}
          {row.by && <span className="text-textMuted"> · {row.by}</span>}
        </span>
      </div>
    );
  }

  const decide = async (allow: boolean) => {
    if (busy) return;
    setBusy(allow ? 'allow' : 'deny');
    setError(null);
    // Twitch's answer folds the row (the AutoMod store's update); only a
    // refusal needs words here.
    const failure = await useAutomodStore.getState().resolve(held.message_id, allow);
    setBusy(null);
    if (failure) setError(failure);
  };

  return (
    <div className="mx-2.5 my-1.5 rounded-[14px] border border-warning/25 bg-warning/10 px-3 py-2.5">
      <div className="mb-1 flex items-center gap-1.5 text-[12px] font-semibold text-warning">
        <Flag size={12} weight="fill" className="flex-shrink-0" />
        <span className="min-w-0">{held.label ?? 'Held by AutoMod'}</span>
      </div>
      <p className="text-[14.5px] leading-snug text-textPrimary [overflow-wrap:anywhere]">
        <span className="font-bold text-accent">{name}</span>: {held.text}
      </p>
      <div className="mt-2 flex gap-2">
        <button
          type="button"
          disabled={busy != null}
          onClick={() => void decide(true)}
          className="flex h-[34px] flex-1 items-center justify-center rounded-[10px] bg-success/15 text-[14px] font-semibold text-success disabled:opacity-60"
        >
          {busy === 'allow' ? 'Allowing…' : 'Allow'}
        </button>
        <button
          type="button"
          disabled={busy != null}
          onClick={() => void decide(false)}
          className="flex h-[34px] flex-1 items-center justify-center rounded-[10px] bg-error/15 text-[14px] font-semibold text-error disabled:opacity-60"
        >
          {busy === 'deny' ? 'Denying…' : 'Deny'}
        </button>
      </div>
      {error && <p className="mt-1.5 text-[12px] text-error">{error}</p>}
    </div>
  );
}

export default memo(HeldMessageRow);
