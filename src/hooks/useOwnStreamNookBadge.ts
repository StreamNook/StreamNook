// The viewer's own StreamNook badge beside their own name, on this screen only
// (`chat_design.show_own_streamnook_badge`, for clean screenshots). Others still
// see it and nothing is written anywhere; Rust applies the same rule to the
// rows and headers it builds (`chat_row_view::own_badge_hidden`). The identity
// editor's previews of what others see keep the badge.
import { useAppStore } from '../stores/AppStore';
import { useSendAccountStore } from '../stores/sendAccountStore';
import { ownBadgeHidden } from '../utils/chatterBadges';

/** True when `memberId` is one of your accounts and you turned your own badge off. */
export function useOwnBadgeHidden(memberId: string | null | undefined): boolean {
  const showOwn = useAppStore((s) => s.settings.chat_design?.show_own_streamnook_badge);
  const currentUserId = useAppStore((s) => s.currentUser?.user_id);
  const linked = useSendAccountStore((s) => !!memberId && s.accounts.some((a) => a.user_id === memberId));
  return ownBadgeHidden(memberId, showOwn, currentUserId, linked && memberId ? [memberId] : []);
}
