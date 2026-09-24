/**
 * Which StreamNook member, if any, is behind a chat identity.
 *
 * Everything StreamNook knows about a member is filed under their Twitch user
 * id: `user_cosmetics`, `user_cosmetic_active`, `user_cosmetic_equipment`,
 * `user_profile_prefs`, `user_numbers`. Chat, meanwhile, identifies people per
 * platform. This is the single place those two spaces meet.
 *
 * ONE owner, deliberately. A second implementation of a key derivation drifts
 * the moment a platform is added, and nothing catches it: both sides are
 * strings, so a mismatch is not a type error, it is a lookup that quietly
 * returns nothing. The rule this file exists to keep is that a Kick user 676 and
 * a Twitch user 676 are different people.
 */

/**
 * Chat user keys follow the persisted convention: a bare id means Twitch, and
 * every other platform is `provider:id`. YouTube ids are CASE SENSITIVE
 * (`UCabc` and `UCABC` are different channels), so nothing here lowercases, and
 * `parseKey` is deliberately not used — its bare-key fallback folds case.
 */
function splitChatKey(chatUserId: string): { provider: string; id: string } {
    const sep = chatUserId.indexOf(':');
    if (sep === -1) return { provider: 'twitch', id: chatUserId };
    return { provider: chatUserId.slice(0, sep), id: chatUserId.slice(sep + 1) };
}

/**
 * `provider:id` -> the Twitch user id of the member who claimed it.
 *
 * Only non-Twitch platforms appear here; a Twitch chatter needs no translation.
 * Filled from the backend, which resolves only the keys actually seen in chat
 * rather than handing every client the whole mapping.
 */
let aliases: ReadonlyMap<string, string> = new Map();

/** Bumped whenever the map changes, so a caller can tell a stale read. */
let aliasVersion = 0;

/**
 * Replace the known aliases.
 *
 * Takes the whole map rather than merging, so a claim that was RELEASED
 * disappears instead of lingering: a member who disconnects their Kick account
 * must stop wearing cosmetics there, and a merge could never express that.
 */
export function setMemberAliases(next: ReadonlyMap<string, string>): void {
    aliases = next;
    aliasVersion += 1;
}

/** Add newly resolved entries without dropping what is already known. */
export function addMemberAliases(entries: Iterable<readonly [string, string]>): void {
    const merged = new Map(aliases);
    let changed = false;
    for (const [key, memberId] of entries) {
        if (merged.get(key) === memberId) continue;
        merged.set(key, memberId);
        changed = true;
    }
    if (!changed) return;
    aliases = merged;
    aliasVersion += 1;
}

export function getMemberAliasVersion(): number {
    return aliasVersion;
}

/** Test seam. Not used by the app. */
export function __resetMemberAliases(): void {
    aliases = new Map();
    aliasVersion = 0;
}

/**
 * The StreamNook member (a Twitch user id) behind a chat-space user key, or null.
 *
 * A bare id is already a Twitch id and passes through untouched, which is what
 * keeps every Twitch render path byte-identical to before this existed. Anything
 * else needs a claim, and `null` means "nobody has claimed this", which is the
 * ordinary case: most chatters are not StreamNook members.
 */
export function memberIdFor(chatUserId: string | undefined | null): string | null {
    if (!chatUserId) return null;
    const { provider, id } = splitChatKey(chatUserId);
    if (provider === 'twitch') return id || null;
    return aliases.get(chatUserId) ?? null;
}

/**
 * Every chat key currently mapping to this member.
 *
 * The inverse direction, needed because several live updates arrive keyed by
 * Twitch id (a member changing their atmosphere, an identity resolving) while
 * the chat store is keyed per platform. Without this, those updates would look
 * for a row under the member's Twitch id and silently miss a member who is only
 * on screen as `kick:12345`.
 *
 * Linear, and that is fine: it runs on rare events, never per message.
 */
export function chatKeysForMember(memberId: string): string[] {
    const keys: string[] = [];
    for (const [chatKey, id] of aliases) {
        if (id === memberId) keys.push(chatKey);
    }
    return keys;
}
