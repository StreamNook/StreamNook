// Which accolades a member can never earn any more. The profile wall leaves
// them off unless already earned, so nobody stares at a medallion they cannot
// reach. Two kinds: the member-number accolades (a number never changes) and
// events that are over.
//
// TypeScript for the same reason as memberNumberNames.ts: the wall is built in
// ProfileOverview and by the site's copy of this rule (`outOfReach` in
// functions/utils/profile/accoladeCatalog.ts in streamnook.app, kept in step
// by hand).

import { memberNumberName } from './memberNumberNames';
import { MAJOR_COLOGNE_ACCOLADE_ID } from '../services/cologneEvent';
import { GRAND_FINALE_ACCOLADE_ID, SEMIQUINCENTENNIAL_ACCOLADE_ID } from '../services/semiquincentennialEvent';

/** A number of two or more digits that reads the same both ways. */
export const isPalindrome = (n: number | null): boolean => {
  if (n === null || n < 10) return false;
  const s = String(n);
  return s === s.split('').reverse().join('');
};

/** Events that are over: their accolades can be held, never earned. */
const ENDED_EVENTS = [MAJOR_COLOGNE_ACCOLADE_ID, SEMIQUINCENTENNIAL_ACCOLADE_ID, GRAND_FINALE_ACCOLADE_ID];

/** Accolade ids this member can never earn now. */
export const outOfReachAccolades = (memberNumber: number | null): Set<string> => {
  const out = new Set<string>(ENDED_EVENTS);
  if (memberNumber === null || memberNumber > 100) out.add('og');
  if (!isPalindrome(memberNumber)) out.add('palindrome');
  if (memberNumberName(memberNumber) === null) out.add('nice');
  return out;
};
