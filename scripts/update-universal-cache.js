/**
 * Universal Cache Updater Script
 * 
 * This script runs via GitHub Actions to:
 * 1. Fetch global badges from Twitch API
 * 2. Fetch metadata for each badge from BadgeBase
 * 3. Update the universal cache manifest
 * 
 * Required GitHub Secrets:
 * - TWITCH_CLIENT_ID: Twitch API client ID
 * - TWITCH_CLIENT_SECRET: Twitch API client secret
 */

const https = require('https');
const fs = require('fs');
const path = require('path');

// Configuration
// UNIVERSAL_CACHE_DIR points at a checkout of the badge-cache branch in CI
// (releases force-push main, which used to erase every cache commit).
const CACHE_DIR = process.env.UNIVERSAL_CACHE_DIR || path.join(__dirname, '..', 'universal-cache', 'main');
const MANIFEST_PATH = path.join(CACHE_DIR, 'manifest.json');

const TWITCH_CLIENT_ID = process.env.TWITCH_CLIENT_ID;
const TWITCH_CLIENT_SECRET = process.env.TWITCH_CLIENT_SECRET;

// Force refresh ALL badges (set via environment variable or CLI arg)
const FORCE_REFRESH_ALL = process.env.FORCE_REFRESH_ALL === 'true' || process.argv.includes('--force-refresh');

// Maximum age in days before forcing a refresh of badge metadata
// Badges older than this will be refreshed to check for updates
const MAX_CACHE_AGE_DAYS = 7;

// Schema version for badge-metadata entries. Bump when scraper output changes
// in a way that makes older entries invalid; older entries are then re-fetched
// on the next run regardless of age.
const METADATA_SCHEMA_VERSION = 2;

// Rate limiting for BadgeBase
const BADGEBASE_DELAY_MS = 300;

/**
 * Make an HTTPS GET request
 */
function httpsGet(url, headers = {}) {
  return new Promise((resolve, reject) => {
    const urlObj = new URL(url);
    const options = {
      hostname: urlObj.hostname,
      path: urlObj.pathname + urlObj.search,
      method: 'GET',
      headers: {
        'User-Agent': 'StreamNook-Universal-Cache-Updater/1.0',
        ...headers,
      },
    };

    const req = https.request(options, (res) => {
      let data = '';
      res.on('data', chunk => data += chunk);
      res.on('end', () => {
        if (res.statusCode >= 200 && res.statusCode < 300) {
          resolve({ data, statusCode: res.statusCode });
        } else {
          reject(new Error(`HTTP ${res.statusCode}: ${data.substring(0, 200)}`));
        }
      });
    });

    req.on('error', reject);
    req.end();
  });
}

/**
 * Make an HTTPS POST request
 */
function httpsPost(url, body = '', headers = {}) {
  return new Promise((resolve, reject) => {
    const urlObj = new URL(url);
    const options = {
      hostname: urlObj.hostname,
      path: urlObj.pathname + urlObj.search,
      method: 'POST',
      headers: {
        'Content-Type': 'application/x-www-form-urlencoded',
        'Content-Length': Buffer.byteLength(body),
        ...headers,
      },
    };

    const req = https.request(options, (res) => {
      let data = '';
      res.on('data', chunk => data += chunk);
      res.on('end', () => {
        if (res.statusCode >= 200 && res.statusCode < 300) {
          resolve({ data, statusCode: res.statusCode });
        } else {
          reject(new Error(`HTTP ${res.statusCode}: ${data.substring(0, 200)}`));
        }
      });
    });

    req.on('error', reject);
    req.write(body);
    req.end();
  });
}

/**
 * Get Twitch app access token
 */
async function getTwitchToken() {
  if (!TWITCH_CLIENT_ID || !TWITCH_CLIENT_SECRET) {
    throw new Error('TWITCH_CLIENT_ID and TWITCH_CLIENT_SECRET environment variables are required');
  }

  console.log('[Twitch] Getting app access token...');

  const body = new URLSearchParams({
    client_id: TWITCH_CLIENT_ID,
    client_secret: TWITCH_CLIENT_SECRET,
    grant_type: 'client_credentials',
  }).toString();

  const response = await httpsPost('https://id.twitch.tv/oauth2/token', body);
  const data = JSON.parse(response.data);

  console.log('[Twitch] Got access token');
  return data.access_token;
}

/**
 * Fetch global badges from Twitch API
 */
async function fetchTwitchBadges(token) {
  console.log('[Twitch] Fetching global badges...');

  const response = await httpsGet('https://api.twitch.tv/helix/chat/badges/global', {
    'Client-Id': TWITCH_CLIENT_ID,
    'Authorization': `Bearer ${token}`,
  });

  const data = JSON.parse(response.data);
  console.log(`[Twitch] Fetched ${data.data.length} badge sets`);
  return data.data;
}

/**
 * Fetch badge metadata from BadgeBase
 */
async function fetchBadgeMetadata(setId, versionId) {
  const url = `https://badgebase.co/badges/${setId}-v${versionId}/`;

  try {
    const response = await httpsGet(url);
    const html = response.data;

    return {
      date_added: extractDateAdded(html),
      usage_stats: extractUsageStats(html),
      more_info: extractMoreInfo(html),
    };
  } catch (error) {
    // Badge might not exist on BadgeBase yet
    return null;
  }
}

/**
 * Extract date added from BadgeBase HTML
 */
function extractDateAdded(html) {
  // Pattern: "Date of addition" followed by a date
  const patterns = [
    /Date of addition[^<]*<\/span>\s*<span[^>]*>([^<]+)/i,
    /Date of addition[^<]*([0-9]{1,2}\s+\w+\s+[0-9]{4})/i,
    /<li[^>]*>.*?Date of addition.*?([0-9]{1,2}\s+\w+\s+[0-9]{4})/is,
  ];

  for (const pattern of patterns) {
    const match = html.match(pattern);
    if (match && match[1]) {
      return match[1].trim();
    }
  }

  return null;
}

/**
 * Extract usage stats from BadgeBase HTML
 */
function extractUsageStats(html) {
  const match = html.match(/(\d+(?:,\d+)*)\s*users?\s*seen\s*with\s*this\s*badge/i);
  if (match) {
    return `${match[1]} users seen with this badge`;
  }
  return null;
}

/**
 * Decode HTML entities (smart quotes/dashes) so more_info is clean text.
 */
function htmlDecodeEntities(s) {
  return String(s)
    .replace(/&#x([0-9a-fA-F]+);/g, (_, h) => String.fromCodePoint(parseInt(h, 16)))
    .replace(/&#(\d+);/g, (_, d) => String.fromCodePoint(parseInt(d, 10)))
    .replace(/&quot;/g, '"')
    .replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&nbsp;/g, ' ')
    .replace(/&rsquo;/g, '’').replace(/&lsquo;/g, '‘')
    .replace(/&ldquo;/g, '“').replace(/&rdquo;/g, '”')
    .replace(/&ndash;/g, '–').replace(/&mdash;/g, '—')
    .replace(/&amp;/g, '&');
}

/**
 * Extract more info from BadgeBase HTML
 */
function extractMoreInfo(html) {
  const match = html.match(/More Info From Us[\s\S]*?<div[^>]*class="[^"]*text[^"]*"[^>]*>([\s\S]*?)<\/div>/i);
  if (match) {
    let text = match[1];

    // Extract ISO timestamps from timezone-converter elements. Two known shapes:
    //   legacy: <span class="timezone-converter" data-original="2025-12-04T15:00:00Z">…</span>
    //   new:    <time class="timezone-converter" datetime="2026-05-21T16:00:00Z">21 May, 16:00 UTC</time>
    // Both attribute orders are accepted so timestamps survive into more_info.
    const converterPatterns = [
      /<(?:span|time)[^>]*\bclass="[^"]*timezone-converter[^"]*"[^>]*\b(?:data-original|datetime)="([^"]*)"[^>]*>[^<]*<\/(?:span|time)>/gi,
      /<(?:span|time)[^>]*\b(?:data-original|datetime)="([^"]*)"[^>]*\bclass="[^"]*timezone-converter[^"]*"[^>]*>[^<]*<\/(?:span|time)>/gi,
    ];
    for (const pattern of converterPatterns) {
      text = text.replace(pattern, '$1');
    }

    // Remove HTML tags, decode entities (smart quotes/dashes), and drop the
    // U+FFFD replacement char so more_info is clean text, not mojibake.
    text = htmlDecodeEntities(text.replace(/<[^>]*>/g, ' ')).replace(/�/g, '').replace(/\s+/g, ' ').trim();

    return text || null;
  }
  return null;
}

/**
 * Sleep for ms milliseconds
 */
function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

/**
 * Get current timestamp in seconds
 */
function getTimestamp() {
  return Math.floor(Date.now() / 1000);
}

/**
 * Load existing manifest or create new one
 */
function loadManifest() {
  if (fs.existsSync(MANIFEST_PATH)) {
    try {
      const data = fs.readFileSync(MANIFEST_PATH, 'utf-8');
      return JSON.parse(data);
    } catch (e) {
      console.log('[Manifest] Failed to load existing manifest, creating new one');
    }
  }
  return {
    version: 1,
    last_sync: null,
    entries: {},
  };
}

/**
 * Save manifest to file
 */
function saveManifest(manifest) {
  fs.mkdirSync(CACHE_DIR, { recursive: true });
  fs.writeFileSync(MANIFEST_PATH, JSON.stringify(manifest, null, 2));
}

/**
 * Main function
 */
async function main() {
  console.log('='.repeat(50));
  console.log('Universal Badge Cache Updater');
  console.log(`Started at: ${new Date().toISOString()}`);
  console.log('='.repeat(50));

  try {
    // Step 1: Get Twitch token
    const token = await getTwitchToken();

    // Step 2: Fetch badges from Twitch
    const badgeSets = await fetchTwitchBadges(token);

    // Step 3: Load existing manifest
    const manifest = loadManifest();
    console.log(`[Manifest] Loaded ${Object.keys(manifest.entries).length} existing entries`);

    // Step 4: Store global badges
    const badgesData = {
      badges: { data: badgeSets },
      cached_at: getTimestamp(),
    };

    manifest.entries['global_badges'] = {
      id: 'global_badges',
      cache_type: 'badge',
      data: badgesData,
      metadata: {
        timestamp: getTimestamp(),
        expiry_days: 7,
        source: 'twitch',
        version: 1,
      },
    };

    // Count badges
    let totalVersions = 0;
    badgeSets.forEach(set => totalVersions += set.versions.length);
    console.log(`[Badges] ${badgeSets.length} sets, ${totalVersions} total versions`);

    // Step 5: Fetch metadata for each badge
    console.log('\n[Metadata] Fetching badge metadata...');
    if (FORCE_REFRESH_ALL) {
      console.log('[Metadata] ⚡ FORCE_REFRESH_ALL is enabled - will refresh ALL badges');
    }

    let fetched = 0;
    let skipped = 0;
    let failed = 0;
    let refreshed = 0;
    let newBadges = [];
    let refreshedBadges = [];

    const currentTimestamp = getTimestamp();
    const maxAgeSeconds = MAX_CACHE_AGE_DAYS * 24 * 60 * 60;

    for (const badgeSet of badgeSets) {
      for (const version of badgeSet.versions) {
        const metadataKey = `metadata:${badgeSet.set_id}-v${version.id}`;

        // Check if we already have this metadata
        const existing = manifest.entries[metadataKey];
        let shouldFetch = true;
        let isRefresh = false;

        if (existing && existing.metadata && existing.data) {
          // Check if we have valid metadata from badgebase
          if (existing.metadata.source === 'badgebase' && existing.data.date_added) {
            // Check if force refresh is enabled
            if (FORCE_REFRESH_ALL) {
              shouldFetch = true;
              isRefresh = true;
            }
            // Entry was written under an older schema; re-fetch so the new
            // scraper output replaces it (e.g. ISO timestamps in more_info).
            else if ((existing.metadata.version || 1) < METADATA_SCHEMA_VERSION) {
              shouldFetch = true;
              isRefresh = true;
            }
            // Check if the cache is older than MAX_CACHE_AGE_DAYS
            else if (existing.metadata.timestamp &&
              (currentTimestamp - existing.metadata.timestamp) > maxAgeSeconds) {
              shouldFetch = true;
              isRefresh = true;
            }
            // Otherwise skip
            else {
              shouldFetch = false;
              skipped++;
            }
          }
        }

        if (!shouldFetch) {
          continue;
        }

        // Fetch metadata from external source
        await sleep(BADGEBASE_DELAY_MS);
        const metadata = await fetchBadgeMetadata(badgeSet.set_id, version.id);

        if (metadata) {
          // Check if more_info has changed (for logging purposes)
          const oldMoreInfo = existing?.data?.more_info;
          const newMoreInfo = metadata.more_info;
          const moreInfoChanged = isRefresh && oldMoreInfo !== newMoreInfo;

          manifest.entries[metadataKey] = {
            id: metadataKey,
            cache_type: 'badge',
            data: metadata,
            metadata: {
              timestamp: getTimestamp(),
              expiry_days: 0, // Never expire
              source: 'badgebase',
              version: METADATA_SCHEMA_VERSION,
            },
          };

          if (isRefresh) {
            refreshed++;
            if (moreInfoChanged) {
              refreshedBadges.push(`${badgeSet.set_id}-v${version.id} (${version.title}) - more_info UPDATED`);
              console.log(`[Metadata] ↻ Refreshed (updated): ${badgeSet.set_id}-v${version.id} (${version.title})`);
            }
          } else {
            fetched++;
            newBadges.push(`${badgeSet.set_id}-v${version.id} (${version.title})`);
            console.log(`[Metadata] ✓ New: ${badgeSet.set_id}-v${version.id} (${version.title})`);
          }
        } else {
          failed++;
          // Don't log every failed badge, we'll summarize at the end
        }
      }
    }

    // Step 6: Assign positions based on date
    console.log('\n[Positions] Sorting badges by date...');
    const metadataEntries = Object.entries(manifest.entries)
      .filter(([_, entry]) => entry.metadata?.source === 'badgebase')
      .map(([id, entry]) => ({ id, entry }));

    // Sort by date (newest first)
    metadataEntries.sort((a, b) => {
      const dateA = parseDate(a.entry.data?.date_added);
      const dateB = parseDate(b.entry.data?.date_added);
      const dateCompare = dateB - dateA;
      
      if (dateCompare !== 0) return dateCompare;

      // Secondary sort: identical to React's locale string comparison
      // id formats: metadata:set_id-v1
      // We strip 'metadata:' and '-v' to mimic `${set_id}-${version_id}`
      const idA = a.id.replace('metadata:', '').replace('-v', '-');
      const idB = b.id.replace('metadata:', '').replace('-v', '-');
      
      return idA.localeCompare(idB);
    });

    // Assign positions
    metadataEntries.forEach(({ id, entry }, index) => {
      manifest.entries[id].position = index;
    });
    console.log(`[Positions] Assigned positions to ${metadataEntries.length} badges`);

    // Step 7: Update sync time and save
    manifest.last_sync = getTimestamp();
    saveManifest(manifest);

    // Summary
    console.log('\n' + '='.repeat(50));
    console.log('Summary:');
    console.log(`  Total badge sets: ${badgeSets.length}`);
    console.log(`  Total versions: ${totalVersions}`);
    console.log(`  Metadata new: ${fetched}`);
    console.log(`  Metadata refreshed: ${refreshed}`);
    console.log(`  Metadata skipped (cached < ${MAX_CACHE_AGE_DAYS} days): ${skipped}`);
    console.log(`  Metadata failed: ${failed}`);
    console.log(`  Manifest entries: ${Object.keys(manifest.entries).length}`);

    if (newBadges.length > 0) {
      console.log('\nNew badges added:');
      newBadges.forEach(b => console.log(`  - ${b}`));
    }

    if (refreshedBadges.length > 0) {
      console.log('\nBadges with updated more_info:');
      refreshedBadges.forEach(b => console.log(`  - ${b}`));
    }

    console.log('\n' + '='.repeat(50));
    console.log(`Completed at: ${new Date().toISOString()}`);

  } catch (error) {
    console.error('\n[ERROR]', error.message);
    process.exit(1);
  }
}

/**
 * Parse date string to timestamp
 * Handles multiple formats:
 * - "12 November 2025" (full format)
 * - "Dec 1-12" or "Dec 1 - 12" (abbreviated month + day range)
 * - "Dec 06 – Dec 07" (full format with month on both sides)
 * - ISO format and other parseable formats
 */
function parseDate(dateStr) {
  if (!dateStr) return 0;
  try {
    // Month name mappings (full and abbreviated)
    const months = {
      'January': 0, 'February': 1, 'March': 2, 'April': 3,
      'May': 4, 'June': 5, 'July': 6, 'August': 7,
      'September': 8, 'October': 9, 'November': 10, 'December': 11,
      'Jan': 0, 'Feb': 1, 'Mar': 2, 'Apr': 3,
      'Jun': 5, 'Jul': 6, 'Aug': 7,
      'Sep': 8, 'Oct': 9, 'Nov': 10, 'Dec': 11
    };

    // Try to match "DD Month YYYY" format (e.g., "12 November 2025")
    const fullMatch = dateStr.match(/(\d{1,2})\s+(\w+)\s+(\d{4})/);
    if (fullMatch) {
      const day = parseInt(fullMatch[1], 10);
      const monthName = fullMatch[2];
      const year = parseInt(fullMatch[3], 10);

      if (months.hasOwnProperty(monthName)) {
        const date = new Date(year, months[monthName], day);
        if (!isNaN(date.getTime())) {
          return date.getTime();
        }
      }
    }

    // Try to match "Mon DD – Mon DD" format (e.g., "Dec 06 – Dec 07") - with en-dash or regular dash
    const fullRangeMatch = dateStr.match(/(\w{3})\s+(\d{1,2})\s*[–-]\s*(\w{3})\s+(\d{1,2})/);
    if (fullRangeMatch) {
      const startMonthAbbrev = fullRangeMatch[1];
      const startDay = parseInt(fullRangeMatch[2], 10);
      const currentYear = new Date().getFullYear();

      if (months.hasOwnProperty(startMonthAbbrev)) {
        const date = new Date(currentYear, months[startMonthAbbrev], startDay);
        if (!isNaN(date.getTime())) {
          return date.getTime();
        }
      }
    }

    // Try to match abbreviated format "Mon D-D" or "Mon D - D" (e.g., "Dec 1-12" or "Dec 1 - 12")
    const abbrevMatch = dateStr.match(/(\w{3})\s+(\d{1,2})\s*[–-]\s*(\d{1,2})(?!\s*\w)/);
    if (abbrevMatch) {
      const monthAbbrev = abbrevMatch[1];
      const startDay = parseInt(abbrevMatch[2], 10);
      const currentYear = new Date().getFullYear();

      if (months.hasOwnProperty(monthAbbrev)) {
        const date = new Date(currentYear, months[monthAbbrev], startDay);
        if (!isNaN(date.getTime())) {
          return date.getTime();
        }
      }
    }

    // Try to match "Month YYYY" format (e.g., "May 2016", "November 2025")
    // IMPORTANT: Must come BEFORE "Mon D" to prevent "2016" being parsed as day 20
    const monthYearMatch = dateStr.match(/^(\w+)\s+(\d{4})$/);
    if (monthYearMatch) {
      const monthName = monthYearMatch[1];
      const year = parseInt(monthYearMatch[2], 10);

      if (months.hasOwnProperty(monthName)) {
        // Use the 1st day of the month for sorting
        const date = new Date(year, months[monthName], 1);
        if (!isNaN(date.getTime())) {
          return date.getTime();
        }
      }
    }

    // Try to match "Mon D" format (e.g., "Dec 1")
    const singleDayMatch = dateStr.match(/(\w{3})\s+(\d{1,2})(?!\s*[–-])/);
    if (singleDayMatch) {
      const monthAbbrev = singleDayMatch[1];
      const day = parseInt(singleDayMatch[2], 10);
      const currentYear = new Date().getFullYear();

      if (months.hasOwnProperty(monthAbbrev)) {
        const date = new Date(currentYear, months[monthAbbrev], day);
        if (!isNaN(date.getTime())) {
          return date.getTime();
        }
      }
    }

    // Fallback: try parsing the date string directly
    const parsed = new Date(dateStr);
    return isNaN(parsed.getTime()) ? 0 : parsed.getTime();
  } catch {
    return 0;
  }
}

// Run
main();
