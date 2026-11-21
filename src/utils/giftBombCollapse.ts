// Shared gift-sub "bomb" collapse rules, used by BOTH the main-chat ingestion
// path (chatConnectionStore) and the overlay render filter (OverlayChat) so the
// two can't drift. A community gift bomb is one `submysterygift` announcement
// ("X is gifting N subs") plus N `subgift` children that share an origin id; the
// anon variants (`anonsubmysterygift` / `anonsubgift`) mirror the same shape.

type TagBag = Map<string, string> | Record<string, string | undefined> | undefined | null;

function readTag(tags: TagBag, key: string): string | undefined {
  if (!tags) return undefined;
  if (tags instanceof Map) return tags.get(key) ?? undefined;
  return (tags as Record<string, string | undefined>)[key];
}

/** The community-gift origin shared by a bomb's announcement and its children.
 *  Tolerates Map tags (post-parseMessage) or plain-object tags (backend/overlay). */
export function giftBombOriginOf(tags: TagBag): string | undefined {
  return readTag(tags, 'msg-param-origin-id') || readTag(tags, 'msg-param-community-gift-id') || undefined;
}

/** The "X is gifting N subs" announcement row (anon or not). */
export function isGiftBombAnnouncement(msgType?: string | null): boolean {
  return msgType === 'submysterygift' || msgType === 'anonsubmysterygift';
}

/** One individual gift within a bomb (anon or not). */
export function isGiftBombChild(msgType?: string | null): boolean {
  return msgType === 'subgift' || msgType === 'anonsubgift';
}

/** The gift that went to the viewer, which never folds into its bomb: as one
 *  name in the announcement's list it went unseen. Rust decides it (chat_rules
 *  `gift_to_me` stamps the row a mention); a gift row carries no words of its
 *  own, so it is a mention only then. */
export function isGiftToViewer(m: { metadata?: { is_mentioned?: boolean } | null } | null | undefined): boolean {
  return m?.metadata?.is_mentioned === true;
}
