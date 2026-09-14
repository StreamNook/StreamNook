// AutoMod queue strip: sits between the messages and the composer for
// moderators, collapsed to one line ("3 held by AutoMod") until opened.
// Each row: who, what, why (category/level or the blocked terms), and
// Allow / Deny, which go to Rust -> Helix. Rows leave on the EventSub update.

import { useEffect, useState } from 'react';
import { Check, ShieldAlert, X, ChevronDown, ChevronUp } from 'lucide-react';
import { useAutomodStore, type HeldMessage } from '../../stores/automodStore';
import { Tooltip } from '../ui/Tooltip';

interface AutomodQueueStripProps {
  /** Lowercase Twitch login of the channel this pane shows. */
  channel: string;
}

function reasonLabel(row: HeldMessage): string {
  if (row.reason === 'blocked_term') {
    return row.terms && row.terms.length > 0 ? `Blocked term: ${row.terms.join(', ')}` : 'Blocked term';
  }
  const cat = row.category ? row.category.replace(/_/g, ' ') : 'AutoMod';
  return row.level ? `${cat} · level ${row.level}` : cat;
}

export default function AutomodQueueStrip({ channel }: AutomodQueueStripProps) {
  const key = channel.toLowerCase();
  const held = useAutomodStore((s) => s.held[key]);
  const seed = useAutomodStore((s) => s.seed);
  const resolve = useAutomodStore((s) => s.resolve);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void seed(key);
  }, [key, seed]);

  const rows = held ?? [];
  if (rows.length === 0) return null;

  const act = async (row: HeldMessage, allow: boolean) => {
    setBusy(row.message_id);
    setError(null);
    const err = await resolve(row.message_id, allow);
    setBusy(null);
    if (err) setError(err);
  };

  return (
    <div className="sn-automod-strip flex-shrink-0">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-textSecondary transition-colors hover:text-textPrimary"
        aria-expanded={open}
      >
        <ShieldAlert size={13} className="flex-shrink-0 text-amber-400" />
        <span className="font-semibold text-textPrimary tabular-nums">{rows.length}</span>
        <span className="min-w-0 flex-1 truncate">
          {rows.length === 1 ? 'message held by AutoMod' : 'messages held by AutoMod'}
          {!open && rows[0] ? ` · ${rows[0].user_name || rows[0].user_login}: ${rows[0].text}` : ''}
        </span>
        {open ? <ChevronDown size={13} /> : <ChevronUp size={13} />}
      </button>
      {open && (
        <ul className="custom-scrollbar max-h-48 overflow-y-auto border-t border-white/5">
          {rows.map((row) => (
            <li key={row.message_id} className="flex items-start gap-2 px-3 py-1.5 text-xs transition-colors hover:bg-white/[0.03]">
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline gap-2">
                  <span className="font-semibold text-textPrimary">{row.user_name || row.user_login}</span>
                  <span className="truncate text-[10px] text-amber-300/80">{reasonLabel(row)}</span>
                </div>
                <div className="break-words text-textPrimary/90">{row.text}</div>
              </div>
              <div className="flex flex-shrink-0 items-center gap-1">
                <Tooltip content="Allow: the message is posted" side="top">
                  <button
                    type="button"
                    disabled={busy === row.message_id}
                    onClick={() => void act(row, true)}
                    className="grid h-6 w-6 place-items-center rounded text-emerald-400 transition-colors hover:bg-emerald-400/15 disabled:opacity-40"
                    aria-label="Allow message"
                  >
                    <Check size={14} />
                  </button>
                </Tooltip>
                <Tooltip content="Deny: the message is dropped" side="top">
                  <button
                    type="button"
                    disabled={busy === row.message_id}
                    onClick={() => void act(row, false)}
                    className="grid h-6 w-6 place-items-center rounded text-error transition-colors hover:bg-error/15 disabled:opacity-40"
                    aria-label="Deny message"
                  >
                    <X size={14} />
                  </button>
                </Tooltip>
              </div>
            </li>
          ))}
        </ul>
      )}
      {error && <div className="px-3 pb-1.5 text-[11px] text-error">{error}</div>}
    </div>
  );
}
