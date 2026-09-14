// Badge navigation cards are built from guesses: the drop campaign supplies one
// name, the prose parser may guess another, and both are then resolved against
// Twitch. Two guesses that read differently can be the same place.

/** The shape the dedupe judges: what kind of card, and what it resolved to. */
export interface ResolvedLink {
    type: string;
    name: string;
    /** Twitch's id for the category, once resolved. Identity, when we have it. */
    categoryId?: string;
}

/**
 * One card per thing, judged on what each link RESOLVED to rather than on how
 * it was written.
 *
 * Candidates are deduped once before resolving, comparing the names as written,
 * and that pass is blind to two spellings of one category: a campaign saying
 * "Grand Theft Auto" and prose saying "Grand Theft Auto V" are different
 * strings that Twitch resolves to a single id, so the panel rendered the
 * category twice, as two cards each fetching its own viewer count. Resolution
 * is what makes them comparable, so this second pass has to run after it.
 *
 * Order is preserved and the first occurrence wins, which keeps the
 * authoritative campaign-supplied link ahead of anything guessed from prose.
 */
export function dedupeResolvedLinks<T extends ResolvedLink>(links: T[]): T[] {
    const seen = new Set<string>();
    return links.filter((link) => {
        const key = `${link.type}:${(link.categoryId || link.name).toLowerCase()}`;
        if (seen.has(key)) return false;
        seen.add(key);
        return true;
    });
}
