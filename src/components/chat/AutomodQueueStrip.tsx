// AutoMod queue strip: sits between the messages and the composer for
// moderators, collapsed to one line ("3 held by AutoMod") until opened.
// Each row: who, what, why (category/level or the blocked terms), and
// Allow / Deny, which go to Rust -> Helix. Rows leave on the EventSub update.

import { useEffect, useState } from 'react';
import { ShieldAlert, ChevronDown, ChevronUp } from 'lucide-react';
import { useAutomodStore, type HeldMessage } from '../../stores/automodStore';
import { Tooltip } from '../ui/Tooltip';
import { ACCENT_BUTTON, ACCENT_FILL, BANNER_STYLE, GLAZE_BANNER, GLAZE_PILL } from '../ui/glazeButtons';

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
    // A glazed card in the chat column, like the streak and resub banners.
    <div className="flex-shrink-0 px-2 pb-2">
      <div className={`${GLAZE_BANNER} overflow-hidden`} style={BANNER_STYLE}>
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="flex h-9 w-full items-center gap-2 px-3 text-left text-xs text-textSecondary transition-colors hover:text-textPrimary"
          aria-expanded={open}
        >
          <ShieldAlert size={14} className="flex-shrink-0 text-amber-400" />
          <span className="font-semibold text-textPrimary tabular-nums">{rows.length}</span>
          <span className="min-w-0 flex-1 truncate">
            {rows.length === 1 ? 'message held by AutoMod' : 'messages held by AutoMod'}
            {!open && rows[0] ? ` · ${rows[0].user_name || rows[0].user_login}: ${rows[0].text}` : ''}
          </span>
          {open ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
        </button>
        {open && (
          // A soft fade at the bottom instead of a hard cut while more scrolls.
          <ul className="custom-scrollbar max-h-64 overflow-y-auto border-t border-white/[0.06] divide-y divide-white/[0.06] pb-1 [mask-image:linear-gradient(to_bottom,black_calc(100%-16px),transparent)]">
            {rows.map((row) => (
              <li key={row.message_id} className="px-3 py-3 text-xs">
                <div className="flex items-baseline gap-2">
                  <span className="font-semibold text-textPrimary">{row.user_name || row.user_login}</span>
                  <span className="truncate text-[11px] text-amber-300/80">{reasonLabel(row)}</span>
                </div>
                <div className="mt-1 break-words leading-relaxed text-textPrimary/90">{row.text}</div>
                <div className="mt-2.5 flex gap-2">
                  <Tooltip content="The message is posted" side="top">
                    <button
                      type="button"
                      disabled={busy === row.message_id}
                      onClick={() => void act(row, true)}
                      className={`${ACCENT_BUTTON} h-7 px-4 !text-xs`}
                      style={ACCENT_FILL}
                    >
                      Allow
                    </button>
                  </Tooltip>
                  <Tooltip content="The message is dropped" side="top">
                    <button
                      type="button"
                      disabled={busy === row.message_id}
                      onClick={() => void act(row, false)}
                      className={`${GLAZE_PILL} h-7 px-4 text-xs`}
                    >
                      Deny
                    </button>
                  </Tooltip>
                </div>
              </li>
            ))}
          </ul>
        )}
        {error && <div className="border-t border-white/[0.06] px-3 py-2 text-xs text-error">{error}</div>}
      </div>
    </div>
  );
}
