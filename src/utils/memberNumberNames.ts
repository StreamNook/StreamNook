// The name a meme member number goes by, shown as the label under the secret
// "Nice" accolade's medallion. It stays one accolade (id `nice`); only the
// label changes with the number, so #69 reads "Nice", #420 "Blaze It" and
// #167 "Six Seven-ish".
//
// TypeScript on purpose: the accolade wall is built in ProfileOverview and,
// for the public member page, by the site's own copy of this table
// (functions/utils/profile/memberNumberNames.ts in streamnook.app, kept in step
// by hand). A fixed lookup inside that render has nothing for Rust to own.
//
// Numbers the chat badge also treats as memes use the badge's words
// (StreamNookBadge.tsx EASTER_EGGS), so the two never disagree.

const EXACT: Readonly<Record<number, string>> = {
  42: 'The Answer',
  67: 'Six Seven',
  69: 'Nice',
  141: 'Going Dark',
  404: 'Not Found',
  420: 'Blaze It',
  666: 'The Beast',
  777: 'Jackpot',
  1337: 'Leet',
  9000: 'Over 9000',
};

/** Close enough to 67 to count: any other number ending in 67. */
const SIX_SEVEN_ISH = 'Six Seven-ish';

/** The accolade label for a member number, or null when it is not a meme number. */
export const memberNumberName = (n: number | null): string | null => {
  if (n === null || !Number.isInteger(n) || n <= 0) return null;
  const exact = EXACT[n];
  if (exact) return exact;
  if (n % 100 === 67) return SIX_SEVEN_ISH;
  return null;
};
