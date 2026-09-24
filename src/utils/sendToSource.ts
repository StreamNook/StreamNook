// Sending one message to one chat source, whichever platform it is on.
//
// Extracted from BlendedChatPane so the main chat panel can route a reply back
// to the platform its parent message came from without a second copy of this
// routing. A second implementation is how the blended composer silently stopped
// delivering once already: it invoked `send_chat_message` with null ids, which
// made the backend skip Helix and fall back to an IRC write that does not
// deliver in that context.

import { invoke } from '@tauri-apps/api/core';
import { sendChannelMessage, injectSystemMessage, systemSourceFor } from '../stores/chatConnectionStore';
import { useAppStore } from '../stores/AppStore';
import type { ProviderId } from '../types/providers';

/** The minimum a caller needs to describe where a message is going. */
export interface ChatSource {
  channel: string;
  provider?: ProviderId;
}

export const sourceProviderOf = (c: ChatSource): ProviderId => c.provider ?? 'twitch';

/** Stable per-source key, shared by the picker, the merge match and this router. */
export const sourceKeyOf = (c: ChatSource) => `${sourceProviderOf(c)}::${c.channel.toLowerCase()}`;

/**
 * Send `text` to one source. With `reply`, route a reply through that provider's
 * own mechanism: Twitch and Kick thread natively off the parent message id;
 * YouTube and TikTok have no threaded reply, so we @mention the recipient instead.
 */
export async function sendToSource(
  c: ChatSource,
  text: string,
  reply?: { parentId: string; parentUser: string },
): Promise<void> {
  const prov = sourceProviderOf(c);
  if (prov === 'twitch') {
    // Route Twitch sends through the shared store path the rest of the app uses.
    // It sends via Helix with the real broadcaster + sender ids AND adds the
    // optimistic copy to the slice, so the message also shows in the feed.
    const cu = useAppStore.getState().currentUser;
    if (!cu?.user_id) return;
    await sendChannelMessage(
      c.channel,
      text,
      {
        username: cu.login || cu.username,
        displayName: cu.display_name || cu.username,
        userId: cu.user_id,
      },
      reply?.parentId,
    );
  } else {
    // YouTube and TikTok live chat have no reply threads, so a reply becomes an
    // @mention.
    const isMentionReply = (prov === 'youtube' || prov === 'tiktok') && !!reply;
    const outcome = await invoke<{
      message_id: string | null;
      is_sent: boolean;
      drop_reason: string | null;
    }>('provider_send_message', {
      provider: prov,
      channel: c.channel.toLowerCase(),
      text: isMentionReply ? `@${reply!.parentUser} ${text}` : text,
      replyTo: isMentionReply ? null : reply?.parentId ?? null,
    });
    // The platform can accept the request and still refuse the message. Say so
    // in the pane and rethrow so the composer can restore what was typed.
    if (outcome && outcome.is_sent === false) {
      const reason = outcome.drop_reason || 'Message not sent';
      const key = `${prov}:${c.channel.toLowerCase()}`;
      injectSystemMessage(key, `Your message was not sent: ${reason}`, undefined, systemSourceFor(key));
      throw new Error(reason);
    }
  }
}
