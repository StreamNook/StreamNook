/**
 * Badge Parsing Service
 * 
 * This service extracts game/category names and special event names from badge
 * descriptions and "more info" sections. This enables deep linking from badge
 * details to category views or drops campaigns.
 */

export interface ParsedBadgeLink {
    type: 'category' | 'drops';
    name: string;
    originalText: string;
}

/**
 * Validate that an extracted name looks like a real game/category name.
 * Returns true if valid, false if it looks like a sentence or invalid text.
 */
function isValidCategoryName(name: string): boolean {
    // Must have some content
    if (!name || name.length < 3) return false;

    // Must not be too long (game names rarely exceed 60 chars)
    if (name.length > 60) return false;

    // Check for words that indicate this is not a specific category
    const invalidWords = [
        'any', 'same', 'the', 'a', 'an', // generic
        'you', 'your', 'need', 'to', 'from', 'between', // sentence words
        'download', 'export', 'share', 'link', 'clip', // action words
        'earn', 'badge', 'exclusive', 'unlock', // badge-related
        'subscribe', 'gift', 'subscription', // unless it's a specific pattern
    ];

    const nameLower = name.toLowerCase();

    // If it's just one of these words, reject
    if (invalidWords.includes(nameLower)) return false;

    // If it contains multiple sentence-like words, it's probably not a category name
    let sentenceWordCount = 0;
    for (const word of invalidWords) {
        if (nameLower.includes(` ${word} `) || nameLower.startsWith(`${word} `) || nameLower.endsWith(` ${word}`)) {
            sentenceWordCount++;
        }
    }
    if (sentenceWordCount >= 2) return false;

    // Check for too many words (game names rarely have more than 8 words)
    const wordCount = name.split(/\s+/).length;
    if (wordCount > 8) return false;

    return true;
}

/**
 * Extract category/game names from badge text.
 * Looks for patterns like:
 * - "in the Tom Clancy's Rainbow Six Siege X category"
 * - "the Marvel Rivals category"
 * - "to a Rainbow Six Siege X streamer"
 * 
 * IMPORTANT: Rejects "any category" patterns as these are not specific categories.
 */
export function extractCategoryName(text: string): ParsedBadgeLink | null {
    if (!text) return null;

    // Normalize smart quotes to regular quotes for matching
    const normalizedText = text
        .replace(/[\u2018\u2019\u201A\u201B]/g, "'")  // Smart single quotes to regular
        .replace(/[\u201C\u201D\u201E\u201F]/g, '"'); // Smart double quotes to regular

    // SKIP: Check if text mentions "any category" - this means it's NOT category-specific
    if (/\bany\s+category\b/i.test(normalizedText)) {
        return null;
    }

    // SKIP: Check if text mentions "from any category" or similar non-specific patterns
    if (/\bfrom\s+any\b/i.test(normalizedText)) {
        return null;
    }

    // Pattern: "in the [Game Name] category" - capture game names with special chars
    // Use non-greedy matching to avoid capturing too much
    // Only match if preceded by word boundary or common prepositions
    const categoryPattern1 = /(?:in|from|streaming)\s+the\s+([A-Za-z0-9][\w\s:'"\-&!.,]{2,50}?)\s+category/i;
    const match1 = normalizedText.match(categoryPattern1);
    if (match1) {
        const name = match1[1].trim();
        if (isValidCategoryName(name)) {
            return {
                type: 'category',
                name: name,
                originalText: match1[0],
            };
        }
    }

    // Pattern: "to [a/an] [Game Name] streamer" - for subscribe/gift patterns
    // Use non-greedy matching with reasonable length limit
    const streamerPattern = /to\s+(?:a|an)\s+([A-Za-z0-9][\w\s:'"\-&!.,]{2,50}?)\s+streamer/i;
    const matchStreamer = normalizedText.match(streamerPattern);
    if (matchStreamer) {
        const name = matchStreamer[1].trim();
        if (isValidCategoryName(name)) {
            return {
                type: 'category',
                name: name,
                originalText: matchStreamer[0],
            };
        }
    }

    // Pattern: "streamer in the [Game Name] category"
    const categoryPattern2 = /streamer\s+in\s+the\s+([A-Za-z0-9][\w\s:'"\-&!.,]{2,50}?)\s+category/i;
    const match2 = normalizedText.match(categoryPattern2);
    if (match2) {
        const name = match2[1].trim();
        if (isValidCategoryName(name)) {
            return {
                type: 'category',
                name: name,
                originalText: match2[0],
            };
        }
    }

    return null;
}

/**
 * Extract special event/drops names from badge text.
 * Looks for patterns like:
 * - "watching the 2025 Streamer Awards"
 * - "the 2025 Streamer Awards"
 * - "watch X minutes of [Event]"
 */
export function extractDropsEventName(text: string): ParsedBadgeLink | null {
    if (!text) return null;

    // Pattern: "watching the [Year] [Event Name]" or "watching the [Event Name]"
    const watchingPattern = /watching\s+the\s+((?:20\d{2}\s+)?[A-Za-z0-9\s'\-&!]+?)(?:\s*!|\s*\.|\s*,|$)/i;
    const matchWatching = text.match(watchingPattern);
    if (matchWatching) {
        const name = matchWatching[1].trim();
        // Make sure it's a meaningful event name, not just random words
        if (name.length > 3 &&
            !name.toLowerCase().includes('stream') &&
            !name.toLowerCase().includes('live') &&
            !/^\d+\s*(minute|hour)/i.test(name)) {
            return {
                type: 'drops',
                name: name,
                originalText: matchWatching[0],
            };
        }
    }

    // Pattern: "watch X minutes/hours of [Event Name]"
    const watchPattern = /watch\s+\d+\s+(?:minute|hour|min|hr)s?\s+(?:of\s+)?(?:the\s+)?(.+?)(?:\s+stream|\s+live|\s+broadcast|[.,;!]|$)/i;
    const match1 = text.match(watchPattern);
    if (match1) {
        const name = match1[1].trim();
        // Clean up and validate the name
        const cleanName = name.replace(/^(the|a|an)\s+/i, '');
        if (cleanName.length > 3 &&
            !cleanName.toLowerCase().includes('twitch') &&
            !cleanName.toLowerCase().includes('badge')) {
            return {
                type: 'drops',
                name: cleanName,
                originalText: match1[0],
            };
        }
    }

    // Pattern: "the [Year] [Event Name]" followed by common endings
    const yearEventPattern = /(?:^|[.\s])the\s+(20\d{2}\s+[A-Za-z0-9\s'\-&!]+?)(?:\s+event|\s+celebration|\s+anniversary|[.,;!]|$)/i;
    const match2 = text.match(yearEventPattern);
    if (match2) {
        const name = match2[1].trim();
        if (name.length > 5 &&
            !name.toLowerCase().includes('twitch') &&
            !name.toLowerCase().includes('badge')) {
            return {
                type: 'drops',
                name: name,
                originalText: match2[0],
            };
        }
    }

    return null;
}

/**
 * Parse badge description and more_info text to find all extractable links.
 * Returns an array of parsed links with their types.
 * Priority: 
 * - moreInfo often has more accurate/complete category names (e.g., "Tom Clancy's Rainbow Six Siege X")
 * - description may have shorter versions (e.g., "Rainbow Six Siege X")
 * - For subscribe/gift badges, category links are preferred over drops
 */
export function parseBadgeForLinks(
    description: string | undefined,
    moreInfo: string | undefined
): ParsedBadgeLink[] {
    const links: ParsedBadgeLink[] = [];
    const seenTypes = new Set<string>();

    const descriptionText = description || '';
    const moreInfoText = moreInfo || '';

    // Check if this is a subscribe/gift badge (should link to category, not drops)
    const isSubscribeBadge = /subscrib|gift/i.test(descriptionText + moreInfoText);

    // Extract categories from both sources
    const categoryFromDesc = extractCategoryName(descriptionText);
    const categoryFromInfo = extractCategoryName(moreInfoText);

    // Choose the better category name (prefer longer/more complete names from moreInfo)
    let bestCategory: ParsedBadgeLink | null = null;

    if (categoryFromDesc && categoryFromInfo) {
        // Both have category names - prefer the longer one (usually more accurate)
        // Also check if one contains the other (the containing one is more complete)
        const descName = categoryFromDesc.name.toLowerCase();
        const infoName = categoryFromInfo.name.toLowerCase();

        if (infoName.includes(descName) || categoryFromInfo.name.length > categoryFromDesc.name.length) {
            // moreInfo has more complete name
            bestCategory = categoryFromInfo;
        } else if (descName.includes(infoName)) {
            // description has more complete name (rare but possible)
            bestCategory = categoryFromDesc;
        } else {
            // Different names - prefer moreInfo as it's usually more official
            bestCategory = categoryFromInfo;
        }
    } else {
        // Only one has a category - use whichever we found
        bestCategory = categoryFromInfo || categoryFromDesc;
    }

    if (bestCategory) {
        links.push(bestCategory);
        seenTypes.add('category');
    }

    // Only look for drops events if this is NOT a subscribe badge
    // or if we didn't find any category links
    if (!isSubscribeBadge || links.length === 0) {
        // Try to extract drops event name from description first
        const dropsFromDesc = extractDropsEventName(descriptionText);
        if (dropsFromDesc && !seenTypes.has('drops')) {
            // Make sure this isn't a duplicate of the category name
            const categoryName = bestCategory?.name.toLowerCase() || '';
            const dropsName = dropsFromDesc.name.toLowerCase();
            if (!categoryName.includes(dropsName) && !dropsName.includes(categoryName)) {
                links.push(dropsFromDesc);
                seenTypes.add('drops');
            }
        }

        // Only check moreInfo for drops if we haven't found any category
        if (!seenTypes.has('category') && !seenTypes.has('drops')) {
            const dropsFromInfo = extractDropsEventName(moreInfoText);
            if (dropsFromInfo) {
                links.push(dropsFromInfo);
                seenTypes.add('drops');
            }
        }
    }

    return links;
}
