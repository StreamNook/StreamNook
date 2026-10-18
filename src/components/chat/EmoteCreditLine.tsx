// The credit lines under an emote's name in its hover card: who made it, and
// where it comes from. Shared by the chat card and the emote menu's card.
//
// The card is only mounted while it shows, so the lookup runs then and never
// for emotes nobody hovers. Rust answers from the sets it holds first, so most
// 7TV / BTTV / FFZ cards fill in the same frame; Twitch asks Twitch once and is
// cached. Two lines are drawn from the first frame (the second blank while the
// answer is on its way), so the card never grows under the cursor.
import { useEffect, useState } from 'react';

import { canCredit, creditLines, fetchEmoteCredit, type EmoteCredit } from '../../services/emoteCredit';

export interface EmoteCreditLineProps {
  provider: string;
  id?: string;
  /** The name the emote is shown under. */
  name: string;
  /** What the set already says, drawn before the lookup answers. */
  ownerHint?: string;
  originalHint?: string;
  channelLogin?: string | null;
  channelId?: string | null;
}

export function EmoteCreditLine({ provider, id, name, ownerHint, originalHint, channelLogin, channelId }: EmoteCreditLineProps) {
  const lookup = canCredit(provider) && !!id;
  const hinted: EmoteCredit | null =
    ownerHint || originalHint
      ? { provider: provider as EmoteCredit['provider'], ownerName: ownerHint, originalName: originalHint }
      : null;
  // Kept with the key it answers, so a card reused for another emote never
  // shows the previous emote's credit.
  const key = `${provider}:${id ?? ''}:${name}`;
  const [answer, setAnswer] = useState<{ key: string; credit: EmoteCredit | null } | null>(null);

  useEffect(() => {
    if (!lookup || !id) return;
    let alive = true;
    void fetchEmoteCredit({ provider, id, name, channel: channelLogin, channelId }).then((credit) => {
      if (alive) setAnswer({ key, credit });
    });
    return () => {
      alive = false;
    };
  }, [lookup, provider, id, name, channelLogin, channelId, key]);

  const settled = answer && answer.key === key ? answer : null;
  const credit = settled ? (settled.credit ?? hinted) : hinted;
  const [first, second] = creditLines(provider, credit);

  return (
    <>
      <span className="text-[10px] text-white/70 leading-tight transition-opacity duration-150">{first}</span>
      {lookup && (
        <span
          className="text-[10px] text-white/55 leading-tight transition-opacity duration-150"
          style={{ opacity: second ? 1 : 0 }}
        >
          {second || ' '}
        </span>
      )}
    </>
  );
}
