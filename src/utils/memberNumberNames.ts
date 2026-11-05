// The name a meme member number goes by, shown as the label under the secret
// "Nice" accolade's medallion. It stays one accolade (id `nice`); only the
// label changes with the number, so #69 reads "Nice", #420 "Blaze It" and
// #167 "Six Seven-ish".
//
// The names live in accolades.catalog.json (`member_number_names`). The
// streamnook.app member page keeps its own copy of this table
// (functions/utils/profile/memberNumberNames.ts) and the shared accolade cases
// check both. Numbers the chat badge also treats as memes use the badge's words
// (StreamNookBadge.tsx EASTER_EGGS), so the two never disagree.
import { CATALOG } from './accoladeCatalogData';

const EXACT = CATALOG.member_number_names.exact;

/** Close enough to 67 to count: any other number ending in 67. */
const SIX_SEVEN_ISH = CATALOG.member_number_names.ends_in_67;

/** The accolade label for a member number, or null when it is not a meme number. */
export const memberNumberName = (n: number | null): string | null => {
  if (n === null || !Number.isInteger(n) || n <= 0) return null;
  const exact = EXACT[String(n)];
  if (exact) return exact;
  if (n % 100 === 67) return SIX_SEVEN_ISH;
  return null;
};
