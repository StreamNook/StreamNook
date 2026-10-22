// Unban requests strip: sits with the AutoMod strip between the messages and
// the composer for moderators, collapsed to one line ("2 unban requests")
// until opened. Each row: who asked, what they wrote, how long ago (Rust's
// words), an optional reply Twitch shows them, and Approve / Deny, which go
// to Rust -> Helix. Rust keeps the pending list (`get_unban_requests`) and
// pushes `unban-requests` on every change; rows leave on that push.

import { useState } from 'react';
import { ChevronDown, ChevronUp, DoorOpen } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { ACCENT_BUTTON, ACCENT_FILL, BANNER_STYLE, GLAZE_BANNER, GLAZE_PILL } from '../ui/glazeButtons';
import { useAppStore } from '../../stores/AppStore';
import { UNBAN_REPLY_MAX, summaryLine, useUnbanRequests, type UnbanRequest } from './unbanRequests';

interface UnbanRequestsStripProps {
  /** Lowercase Twitch login of the channel this pane shows. */
  channel: string;
  /** The channel's Twitch user id. */
  channelId: string;
}

export default function UnbanRequestsStrip({ channel, channelId }: UnbanRequestsStripProps) {
  const { rows, resolve } = useUnbanRequests(channelId, channel, !!channelId);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [replies, setReplies] = useState<Record<string, string>>({});

  if (rows.length === 0) return null;

  const act = async (row: UnbanRequest, approve: boolean) => {
    setBusy(row.id);
    const err = await resolve(row.id, approve, replies[row.id] ?? '');
    setBusy(null);
    // A toast, not a line here: a refusal can empty the list and unmount
    // the strip before a line would show.
    if (err) useAppStore.getState().addToast(err, 'error');
    else
      setReplies((cur) => {
        const next = { ...cur };
        delete next[row.id];
        return next;
      });
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
          <DoorOpen size={14} className="flex-shrink-0 text-info" />
          <span className="font-semibold text-textPrimary tabular-nums">{rows.length}</span>
          <span className="min-w-0 flex-1 truncate">{summaryLine(rows, open)}</span>
          {open ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
        </button>
        {open && (
          // A soft fade at the bottom instead of a hard cut while more scrolls.
          <ul className="custom-scrollbar max-h-72 overflow-y-auto border-t border-white/[0.06] divide-y divide-white/[0.06] pb-1 [mask-image:linear-gradient(to_bottom,black_calc(100%-16px),transparent)]">
            {rows.map((row) => (
              <li key={row.id} className="px-3 py-3 text-xs">
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
        )}
      </div>
    </div>
  );
}
