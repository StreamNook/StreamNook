import { useRef, useState } from 'react';
import { Check, ChevronDown, User } from 'lucide-react';
import { useSendAccountStore } from '../stores/sendAccountStore';
import { Tooltip } from './ui/Tooltip';
import { SevenTvAvatar } from './SevenTvAvatar';
import { useOutsidePress } from '../hooks/useOutsidePress';

/** A linked Twitch account's face; its Twitch id draws its 7TV picture. */
function Avatar({ url, size, userId }: { url: string | null; size: number; userId: string }) {
  if (url) {
    return (
      <SevenTvAvatar
        cosmeticsId={userId}
        px={size}
        src={url}
        alt=""
        className="rounded-full object-cover flex-shrink-0"
        style={{ width: size, height: size }}
      />
    );
  }
  return (
    <div
      className="rounded-full bg-white/10 flex items-center justify-center flex-shrink-0"
      style={{ width: size, height: size }}
    >
      <User size={Math.round(size * 0.6)} className="text-textMuted" />
    </div>
  );
}

/**
 * Picker for choosing which linked account a chat message is sent from. Renders
 * nothing unless 2+ accounts are linked, so single-account users see no change.
 * Lives just to the right of the emote button inside the chat input.
 */
export default function SendAsPicker() {
  const accounts = useSendAccountStore((s) => s.accounts);
  const sendAsId = useSendAccountStore((s) => s.sendAsId);
  const setSendAsId = useSendAccountStore((s) => s.setSendAsId);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const markInside = useOutsidePress(open, [ref], () => setOpen(false));

  if (accounts.length < 2) return null;

  const primary = accounts.find((a) => a.is_primary) ?? accounts[0];
  const selected = accounts.find((a) => a.user_id === sendAsId) ?? primary;
  const isAlt = !selected.is_primary;

  return (
    <div ref={ref} onPointerDownCapture={markInside} className="relative">
      <Tooltip content={`Sending as @${selected.login}`} side="top">
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-label="Choose which account to send from"
          className={`flex items-center gap-0.5 rounded-full p-0.5 transition-colors ${
            isAlt ? 'ring-1 ring-accent' : 'hover:bg-white/10'
          }`}
        >
          <Avatar url={selected.avatar_url} size={20} userId={selected.user_id} />
          <ChevronDown size={11} className="text-textMuted" />
        </button>
      </Tooltip>

      {open && (
        <div className="glass-flyout rounded-xl absolute bottom-full left-0 mb-2 w-56 max-h-72 overflow-y-auto scrollbar-thin z-50">
          <div className="px-2 py-1 text-[11px] font-semibold uppercase tracking-wider text-white/50">
            Send messages as
          </div>
          {accounts.map((account) => {
            const active = account.user_id === selected.user_id;
            return (
              <button
                key={account.user_id}
                type="button"
                onClick={() => {
                  setSendAsId(account.user_id);
                  setOpen(false);
                }}
                className={`w-full flex items-center gap-2.5 rounded-lg px-2 py-2 text-left transition-colors ${
                  active ? 'bg-white/[0.08]' : 'hover:bg-white/[0.06]'
                }`}
              >
                <Avatar url={account.avatar_url} size={26} userId={account.user_id} />
                <div className="min-w-0 flex-1">
                  <div className="text-sm text-textPrimary truncate">
                    {account.display_name || account.login}
                  </div>
                  <div className="text-[11px] text-textSecondary truncate">
                    @{account.login}
                    {account.is_primary ? ' · main' : ''}
                  </div>
                </div>
                {active && <Check size={15} className="text-accent flex-shrink-0" />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
