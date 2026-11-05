// Which accolades a member can never earn any more. The profile wall leaves
// them off unless already held, so nobody stares at a medallion they cannot
// reach. Two kinds: the accolades decided by the member number (OG, Palindrome
// and Nice; a number never changes) and the ones marked `ended` in
// accolades.catalog.json (events that are over).
//
// The streamnook.app member page keeps its own copy of this rule
// (`outOfReach` in functions/utils/profile/accoladeCatalog.ts) and the shared
// accolade cases check both.
import { CATALOG } from './accoladeCatalogData';
import { memberNumberName } from './memberNumberNames';

/** A number of two or more digits that reads the same both ways. */
export const isPalindrome = (n: number | null): boolean => {
  if (n === null || n < 10) return false;
  const s = String(n);
  return s === s.split('').reverse().join('');
};

/** Accolade ids this member can never earn now. */
export const outOfReachAccolades = (memberNumber: number | null): Set<string> => {
  const out = new Set<string>();
  for (const a of CATALOG.accolades) {
    if (a.ended) {
      out.add(a.id);
      continue;
    }
    if (!('special' in a.rule)) continue;
    const special = a.rule.special;
    if (special === 'first_hundred' && (memberNumber === null || memberNumber > 100)) out.add(a.id);
    if (special === 'palindrome' && !isPalindrome(memberNumber)) out.add(a.id);
    if (special === 'nice' && memberNumberName(memberNumber) === null) out.add(a.id);
  }
  return out;
};
