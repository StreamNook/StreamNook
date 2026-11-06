// The Tools menu's Unban requests page, which takes the menu's body like
// Blocked terms. Each row: who asked, what they wrote, how long ago (Rust's
// words), an optional reply Twitch shows them, and Approve / Deny, which go
// to Rust -> Helix. The menu holds the list (`useUnbanRequests`) so its
// Moderation row and this page count the same rows; a row leaves on Rust's
// `unban-requests` push.

import { useState } from 'react';
import { Tooltip } from '../ui/Tooltip';
import { ACCENT_BUTTON, ACCENT_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import { useAppStore } from '../../stores/AppStore';
import { UNBAN_REPLY_MAX, type UnbanRequest } from './unbanRequests';

interface UnbanRequestsPageProps {
  rows: UnbanRequest[];
  /** Answers one request; resolves to Twitch's refusal, or null. */
  resolve: (requestId: string, approve: boolean, reply: string) => Promise<string | null>;
}

export default function UnbanRequestsPage({ rows, resolve }: UnbanRequestsPageProps) {
  const [busy, setBusy] = useState<string | null>(null);
  const [replies, setReplies] = useState<Record<string, string>>({});

  const act = async (row: UnbanRequest, approve: boolean) => {
    setBusy(row.id);
    const err = await resolve(row.id, approve, replies[row.id] ?? '');
    setBusy(null);
    if (err) useAppStore.getState().addToast(err, 'error');
    else
      setReplies((cur) => {
        const next = { ...cur };
        delete next[row.id];
        return next;
      });
  };

  // The last one answered while the page is open: say so rather than
  // dropping the moderator back to the menu.
  if (rows.length === 0) {
    return <p className="px-4 py-4 text-xs text-textMuted">No one is asking to be unbanned right now.</p>;
  }

  return (
    // A soft fade at the bottom instead of a hard cut while more scrolls.
    <ul className="flex-1 min-h-0 custom-scrollbar overflow-y-auto divide-y divide-white/[0.06] pb-1 [mask-image:linear-gradient(to_bottom,black_calc(100%-16px),transparent)]">
      {rows.map((row) => (
        <li key={row.id} className="px-4 py-3 text-xs">
          <div className="flex items-baseline gap-2">
            <span className="font-semibold text-textPrimary">{row.user_name || row.user_login}</span>
            <span className="truncate text-[11px] text-textMuted">{row.age}</span>
          </div>
          {row.text && <div className="mt-1 break-words leading-relaxed text-textPrimary/90">{row.text}</div>}
          <input
            value={replies[row.id] ?? ''}
            onChange={(e) => setReplies((cur) => ({ ...cur, [row.id]: e.target.value }))}
            maxLength={UNBAN_REPLY_MAX}
            disabled={busy === row.id}
            placeholder="Reply they'll see (optional)"
            aria-label={`Reply to ${row.user_name || row.user_login}`}
            className="glass-input mt-2.5 h-8 w-full min-w-0 rounded-lg px-3 text-xs text-textPrimary placeholder-textSecondary"
          />
          <div className="mt-2.5 flex gap-2">
            <Tooltip content="They're unbanned" side="top">
              <button
                type="button"
                disabled={busy === row.id}
                onClick={() => void act(row, true)}
                className={`${ACCENT_BUTTON} h-7 px-4 !text-xs`}
                style={ACCENT_FILL}
              >
                Approve
              </button>
            </Tooltip>
            <Tooltip content="They stay banned" side="top">
              <button
                type="button"
                disabled={busy === row.id}
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
  );
}
