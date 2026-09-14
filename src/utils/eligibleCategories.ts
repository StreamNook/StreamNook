// Badge copy sometimes spells out exactly where a badge can be earned:
//
//   Full list of eligible categories:
//   Pokémon FireRed/LeafGreen, Pokémon GO, Just Chatting, DJs, Art,
//   Animals, Aquariums, and Zoos, Co-working & Studying, and Makers & Crafting
//
// That list is the most useful thing on the page and it was rendered as flat
// prose. Turning it into real categories is not a split on commas, because a
// category name can contain its own commas: "Animals, Aquariums, and Zoos" is
// ONE Twitch category, and — measured against Twitch — "Animals" and
// "Aquariums" are ALSO real categories in their own right. So a splitter that
// resolves the short pieces first is not merely imprecise, it confidently
// produces the wrong three categories and never notices.
//
// The only way to read it correctly is to try the longest run of pieces first
// and let Twitch say which one is real. This file does the splitting and
// proposes the runs; the caller resolves them.

/** One candidate reading of the list at a given position. */
export interface CategoryRun {
    /** The text to test against Twitch, exactly as the copy wrote it. */
    name: string;
    /** How many comma-separated pieces this reading consumes. */
    pieces: number;
}

/** Longest a single category name can run across commas ("A, B, and C"). */
const MAX_RUN = 4;

// The heading that introduces an explicit list. Deliberately narrow: prose that
// merely mentions categories is guesswork, while a labelled list is a promise.
const LIST_HEADING = /(?:full list of\s+)?eligible categories\s*:/i;

// A labelled line that ends the list ("Evolution:", "Official description:"),
// a blank line, or a URL. Badge copy runs these together with no blank line
// between, so stopping at the paragraph break alone is not enough.
const LIST_END = /\n\s*\n|\n(?=[A-Z][A-Za-z' -]{2,30}:\s)|\bhttps?:\/\//;

/**
 * The list text a badge names its eligible categories in, or null when the copy
 * doesn't spell one out.
 */
export function eligibleCategoryList(text: string): string | null {
    if (!text) return null;
    const heading = LIST_HEADING.exec(text);
    if (!heading) return null;

    const rest = text.slice(heading.index + heading[0].length);
    const end = LIST_END.exec(rest);
    const list = (end ? rest.slice(0, end.index) : rest).trim();
    return list.length > 0 ? list : null;
}

/**
 * Every reading of the list, position by position, longest candidate first.
 *
 * The caller walks it in order: at each position take the first candidate
 * Twitch recognises, then skip that candidate's `pieces` entries. Trying the
 * longest first is what makes "Animals, Aquariums, and Zoos" win over the
 * "Animals" it starts with.
 */
export function categoryRuns(text: string): CategoryRun[][] {
    const list = eligibleCategoryList(text);
    if (!list) return [];

    // Split on commas only. "and" is left inside the pieces because it belongs
    // to the name as often as it separates two of them ("Animals, Aquariums,
    // and Zoos" vs "Sports, Music, and Art"), and a joined run has to be able
    // to reproduce the original text verbatim.
    const pieces: { text: string; start: number; end: number }[] = [];
    let cursor = 0;
    for (const raw of list.split(',')) {
        const start = cursor;
        cursor += raw.length + 1; // + the comma
        const trimmed = raw.trim();
        if (trimmed) pieces.push({ text: trimmed, start, end: start + raw.length });
    }

    return pieces.map((_, i) => {
        const runs: CategoryRun[] = [];
        for (let len = Math.min(MAX_RUN, pieces.length - i); len >= 1; len--) {
            const joined = list.slice(pieces[i].start, pieces[i + len - 1].end).trim();
            // A run of one piece may still carry a leading "and" from the copy's
            // final item; the name itself never does.
            runs.push({ name: len === 1 ? stripLeadingAnd(joined) : joined, pieces: len });
        }
        return runs;
    });
}

/** "and Makers & Crafting" -> "Makers & Crafting". */
export function stripLeadingAnd(name: string): string {
    return name.replace(/^and\s+/i, '').trim();
}

/**
 * Reads the list using `exists`, which answers whether a name is a real
 * category. Returns the names in the order the copy wrote them, deduped.
 *
 * Kept separate from the resolving so it can be tested without the network.
 */
export function readCategoryList(text: string, exists: (name: string) => boolean): string[] {
    const positions = categoryRuns(text);
    const out: string[] = [];
    const seen = new Set<string>();

    let i = 0;
    while (i < positions.length) {
        const hit = positions[i].find((run) => exists(run.name));
        if (!hit) {
            i += 1;
            continue;
        }
        const key = hit.name.toLowerCase();
        if (!seen.has(key)) {
            seen.add(key);
            out.push(hit.name);
        }
        i += hit.pieces;
    }
    return out;
}

/** Every name worth asking Twitch about, deduped, for a single batched lookup. */
export function categoryLookupNames(text: string): string[] {
    const seen = new Set<string>();
    const names: string[] = [];
    for (const runs of categoryRuns(text)) {
        for (const run of runs) {
            const key = run.name.toLowerCase();
            if (!seen.has(key)) {
                seen.add(key);
                names.push(run.name);
            }
        }
    }
    return names;
}
