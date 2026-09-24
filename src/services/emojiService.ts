/**
 * Emoji Service - Converts Unicode emojis to iOS-style emoji images
 * Uses Tauri proxy to fetch CDN-hosted Apple emoji images (bypasses tracking prevention)
 * Emoji shortcode conversion is now handled by Rust backend for zero JS heap allocation
 */
import { invoke } from '@tauri-apps/api/core';

import { Logger } from '../utils/logger';
import { LruMap } from './cosmeticsCache';
// LRU-bounded cache for proxied emoji URLs (codepoint -> data URL). 256 entries
// × ~5 KB per Apple emoji = ~1.3 MB ceiling. Twitch chat uses emojis sparingly
// in practice so this rarely fills; the cap protects against an edge-case
// emoji-heavy session pinning 15-20 MB indefinitely in the JS heap.
const proxiedEmojiCache = new LruMap<string, string>(256);

// Pending emoji fetches (to avoid duplicate requests)
const pendingEmojiFetches = new Set<string>();

// Cache for failed emoji fetches (codepoints that returned 404)
// These are permanently excluded from re-fetching
const failedEmojiCodepoints = new Set<string>();

// Queue for batch emoji caching
let emojiCacheQueue: string[] = [];
let emojiCacheFlushTimeout: ReturnType<typeof setTimeout> | null = null;

/**
 * Queue an emoji codepoint for background caching via Tauri proxy
 * This fetches the image through Rust to bypass tracking prevention
 */
export function queueEmojiForCaching(codepoint: string) {
    // Skip if already cached, pending, or previously failed
    if (proxiedEmojiCache.has(codepoint) || pendingEmojiFetches.has(codepoint) || failedEmojiCodepoints.has(codepoint)) {
        return;
    }

    emojiCacheQueue.push(codepoint);
    pendingEmojiFetches.add(codepoint);

    // Debounce: flush queue after 50ms of inactivity
    if (emojiCacheFlushTimeout) {
        clearTimeout(emojiCacheFlushTimeout);
    }
    emojiCacheFlushTimeout = setTimeout(flushEmojiCacheQueue, 50);
}

/**
 * Flush the emoji cache queue - fetch all pending emojis via Tauri proxy
 */
async function flushEmojiCacheQueue() {
    const queue = [...emojiCacheQueue];
    emojiCacheQueue = [];
    emojiCacheFlushTimeout = null;

    // Fetch in parallel with limited concurrency
    const batchSize = 10;
    for (let i = 0; i < queue.length; i += batchSize) {
        const batch = queue.slice(i, i + batchSize);
        await Promise.all(batch.map(async (codepoint) => {
            try {
                const dataUrl = await invoke<string>('get_emoji_image', { codepoint });
                proxiedEmojiCache.set(codepoint, dataUrl);
            } catch (error) {
                // Add to failed cache to prevent re-queueing
                failedEmojiCodepoints.add(codepoint);
                Logger.warn(`[EmojiService] Failed to cache emoji ${codepoint} (will not retry):`, error);
            } finally {
                pendingEmojiFetches.delete(codepoint);
            }
        }));
    }
}

/**
 * Get the cached emoji URL if available, otherwise return the CDN URL
 * and queue the emoji for background caching
 */
export function getCachedEmojiUrl(emoji: string, cdnUrl: string): string {
    const codepoint = emojiToCodepoint(emoji);
    
    // If cached, return the data URL
    if (proxiedEmojiCache.has(codepoint)) {
        return proxiedEmojiCache.get(codepoint)!;
    }
    
    // Queue for caching and return CDN URL for now (will be cached for next render)
    queueEmojiForCaching(codepoint);
    return cdnUrl;
}

// Regular expression to match emoji characters
// This regex covers most common emojis including:
// - Basic emojis (😀-🙏)
// - Skin tone modifiers
// - ZWJ sequences (family, profession emojis)
// - Regional indicators (flags)
// - Keycap emojis
//
// A flag is two regional-indicator letters, and each letter on its own has
// Emoji_Presentation, so the pair has to be tried first and a lone letter kept
// out of the general alternative. Otherwise a flag splits into its two
// letters (1f1e7 and 1f1f7 for Brazil), neither of which has an image.
const EMOJI_REGEX = /\p{Regional_Indicator}{2}|(?:(?!\p{Regional_Indicator})\p{Emoji_Presentation}|\p{Emoji}\uFE0F)(?:\p{Emoji_Modifier})?(?:\u200D(?:\p{Emoji_Presentation}|\p{Emoji}\uFE0F)(?:\p{Emoji_Modifier})?)*/gu;

// CDN URL for Apple emoji images
// Using jsDelivr CDN with emoji-datasource-apple package
// 64px is the highest resolution available in the package
const APPLE_EMOJI_CDN = 'https://cdn.jsdelivr.net/npm/emoji-datasource-apple@15.1.2/img/apple/64';

/**
 * Converts a single emoji character/sequence to its hex codepoint representation
 * Used to build the image URL
 */
export function emojiToCodepoint(emoji: string): string {
    const codepoints: string[] = [];

    for (const char of emoji) {
        const codepoint = char.codePointAt(0);
        if (codepoint !== undefined) {
            // Skip variation selector-16 (FE0F) as it's not always in filenames
            if (codepoint !== 0xFE0F) {
                codepoints.push(codepoint.toString(16).toLowerCase());
            }
        }
    }

    return codepoints.join('-');
}

export type EmojiStyle = 'system' | 'apple' | 'google' | 'twitter' | 'facebook';

const VENDOR_EMOJI_CDN: Record<string, string> = {
    apple: 'https://cdn.jsdelivr.net/npm/emoji-datasource-apple@15.1.2/img/apple/64',
    google: 'https://cdn.jsdelivr.net/npm/emoji-datasource-google@15.1.2/img/google/64',
    facebook: 'https://cdn.jsdelivr.net/npm/emoji-datasource-facebook@15.1.2/img/facebook/64',
};

/**
 * The image for an emoji in a chosen vendor set, or null for the system style
 * (draw the glyph) or an unknown style. Twitter renders from Twemoji's SVG,
 * whose filenames drop FE0F; the raster sets keep it.
 */
export function vendorEmojiUrl(emoji: string, style: string): string | null {
    if (style === 'system') return null;
    const cps = [...emoji].map((c) => c.codePointAt(0)!);
    if (style === 'twitter') {
        const cp = cps.filter((c) => c !== 0xfe0f).map((c) => c.toString(16)).join('-');
        return `https://cdn.jsdelivr.net/gh/jdecked/twemoji@15.1.0/assets/svg/${cp}.svg`;
    }
    const base = VENDOR_EMOJI_CDN[style];
    if (!base) return null;
    return `${base}/${cps.map((c) => c.toString(16)).join('-')}.png`;
}

/**
 * Gets the Apple emoji image URL for a given emoji
 */
export function getAppleEmojiUrl(emoji: string): string {
    const codepoint = emojiToCodepoint(emoji);
    return `${APPLE_EMOJI_CDN}/${codepoint}.png`;
}

/**
 * Synchronously parses text for Unicode emojis
 * Returns an array of segments with emojis and text separated
 * Note: Does NOT convert shortcodes (that's async). For optimistic messages only.
 */
export function parseEmojisSync(text: string): EmojiSegment[] {
    if (!text) {
        return [];
    }

    // Reset regex lastIndex
    EMOJI_REGEX.lastIndex = 0;

    const segments: EmojiSegment[] = [];
    let lastIndex = 0;
    let match: RegExpExecArray | null;

    while ((match = EMOJI_REGEX.exec(text)) !== null) {
        // Add text before the emoji
        if (match.index > lastIndex) {
            segments.push({
                type: 'text',
                content: text.substring(lastIndex, match.index),
            });
        }

        // Add the emoji with Apple CDN URL
        const emoji = match[0];
        segments.push({
            type: 'emoji',
            content: emoji,
            emojiUrl: getAppleEmojiUrl(emoji),
        });

        lastIndex = match.index + emoji.length;
    }

    // Add remaining text after the last emoji
    if (lastIndex < text.length) {
        segments.push({
            type: 'text',
            content: text.substring(lastIndex),
        });
    }

    return segments.length > 0 ? segments : [{ type: 'text', content: text }];
}

/**
 * Replaces emoji shortcodes in text with their unicode equivalents
 * Only matches shortcodes wrapped in colons like :smiley: or :heart:
 * Now offloaded to Rust backend for zero JS heap allocation
 */
async function replaceShortcodes(text: string): Promise<string> {
    if (!text) return text;

    try {
        // Call Rust backend for emoji shortcode conversion
        return await invoke<string>('convert_emoji_shortcodes', { text });
    } catch (error) {
        Logger.warn('Failed to convert emoji shortcodes via Rust, returning original text:', error);
        return text;
    }
}

/**
 * Parses text and returns segments with emojis separated
 * Returns an array of objects with type 'text' or 'emoji'
 */
export interface EmojiSegment {
    type: 'text' | 'emoji';
    content: string;
    emojiUrl?: string;
}

/**
 * Gets the Apple emoji image URL through Tauri's proxy
 * This bypasses browser tracking prevention by using Tauri's HTTP client
 * Returns a base64 data URL that can be used directly in img src
 */
export async function getProxiedEmojiUrl(emoji: string): Promise<string> {
    const codepoint = emojiToCodepoint(emoji);

    // Check frontend cache first
    if (proxiedEmojiCache.has(codepoint)) {
        return proxiedEmojiCache.get(codepoint)!;
    }

    try {
        // Fetch through Tauri proxy (cached in Rust backend)
        const dataUrl = await invoke<string>('get_emoji_image', { codepoint });

        // Cache in frontend
        proxiedEmojiCache.set(codepoint, dataUrl);

        return dataUrl;
    } catch (error) {
        Logger.warn(`Failed to fetch emoji via proxy: ${codepoint}`, error);
        // Return the native emoji as fallback
        return emoji;
    }
}

/**
 * Parses text and returns segments with emojis separated, using proxied URLs
 * This is an async version that fetches emoji images through Tauri's proxy
 */
export async function parseEmojisProxied(text: string): Promise<EmojiSegment[]> {
    if (!text) {
        return [];
    }

    // First replace any shortcodes with actual unicode emojis
    const processedText = await replaceShortcodes(text);

    // Reset regex lastIndex
    EMOJI_REGEX.lastIndex = 0;

    const segments: EmojiSegment[] = [];
    const emojiPromises: Array<{ index: number; emoji: string; promise: Promise<string> }> = [];
    let lastIndex = 0;
    let match: RegExpExecArray | null;
    let segmentIndex = 0;

    while ((match = EMOJI_REGEX.exec(processedText)) !== null) {
        // Add text before the emoji
        if (match.index > lastIndex) {
            segments.push({
                type: 'text',
                content: processedText.substring(lastIndex, match.index),
            });
            segmentIndex++;
        }

        // Add placeholder for the emoji (will be filled with proxied URL)
        const emoji = match[0];
        const emojiSegmentIndex = segmentIndex;
        segments.push({
            type: 'emoji',
            content: emoji,
            emojiUrl: emoji, // Temporary, will be replaced
        });

        // Start fetching the proxied URL
        emojiPromises.push({
            index: emojiSegmentIndex,
            emoji,
            promise: getProxiedEmojiUrl(emoji),
        });

        segmentIndex++;
        lastIndex = match.index + emoji.length;
    }

    // Add remaining text after the last emoji
    if (lastIndex < processedText.length) {
        segments.push({
            type: 'text',
            content: processedText.substring(lastIndex),
        });
    }

    // Wait for all emoji URLs to be fetched
    const results = await Promise.all(emojiPromises.map(p => p.promise));

    // Update emoji segments with proxied URLs
    emojiPromises.forEach((p, i) => {
        if (segments[p.index]) {
            segments[p.index].emojiUrl = results[i];
        }
    });

    return segments.length > 0 ? segments : [{ type: 'text', content: processedText }];
}
