// Badge window display helpers.
//
// Resolving a badge's earn window (campaign timestamps, stamps in the copy,
// prose like "Dec 19 – Jan 01") lives in Rust, services/badge_window.rs; the
// runs it produces are read against the clock by windowStatusAt in
// services/badgeStanding.ts. What remains here is formatting for display.

export type BadgeWindowStatus = 'available' | 'coming-soon' | 'expired';

const ISO_STAMP = String.raw`\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2})?Z?`;

/** Scraped copy arrives with entities intact, which breaks the dash regexes. */
export function decodeHtmlEntities(text: string): string {
  let result = text;

  result = result.replace(/&#(\d+);/g, (_match, dec) => String.fromCharCode(parseInt(dec, 10)));
  result = result.replace(/&#x([0-9a-fA-F]+);/g, (_match, hex) =>
    String.fromCharCode(parseInt(hex, 16))
  );

  const entities: Record<string, string> = {
    '&amp;': '&',
    '&lt;': '<',
    '&gt;': '>',
    '&quot;': '"',
    '&apos;': "'",
    '&nbsp;': ' ',
    '&ndash;': '–',
    '&mdash;': '—',
  };
  for (const [entity, char] of Object.entries(entities)) {
    result = result.split(entity).join(char);
  }
  return result;
}

/**
 * Render a badge's `date_info` for display. Any ISO stamp in it becomes local
 * wall-clock text; prose windows ("Dec 1-12") are already readable and pass
 * through untouched.
 *
 * Sources emit these stamps in UTC, so a raw one is not just machine-looking,
 * it tells the reader the wrong time. A bare stamp with no zone suffix is
 * therefore read as UTC rather than as local. The year is shown only when it is
 * not the current one, which keeps the common case short enough for a toast.
 */
export function formatBadgeDateInfo(dateInfo?: string | null, now: number = Date.now()): string {
  if (!dateInfo) return '';
  const currentYear = new Date(now).getFullYear();
  return decodeHtmlEntities(dateInfo)
    .replace(new RegExp(ISO_STAMP, 'g'), (stamp) => {
      const date = new Date(/[Zz]$/.test(stamp) ? stamp : `${stamp}Z`);
      if (isNaN(date.getTime())) return stamp;
      return date.toLocaleString(undefined, {
        month: 'short',
        day: 'numeric',
        ...(date.getFullYear() === currentYear ? {} : { year: 'numeric' }),
        hour: 'numeric',
        minute: '2-digit',
      });
    })
    .trim();
}
