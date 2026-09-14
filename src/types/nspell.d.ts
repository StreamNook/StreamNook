// nspell ships no TypeScript types. Only the surface the spell-check worker
// actually calls is declared here — the constructor plus the two lookups.
declare module 'nspell' {
  interface NSpell {
    /** True when the word is spelled correctly. */
    correct(word: string): boolean;
    /** Ranked corrections, best first. Empty when nothing is close enough. */
    suggest(word: string): string[];
  }

  interface NSpellDictionary {
    /** Hunspell affix file (.aff) contents. */
    aff: string;
    /** Hunspell dictionary file (.dic) contents. */
    dic: string;
  }

  // nspell is CommonJS (`module.exports = NSpell`). Declared as a default
  // export because that is what the bundler's interop hands us at runtime, and
  // `export =` would need esModuleInterop, which this tsconfig does not set.
  export default function nspell(dictionary: NSpellDictionary): NSpell;
}
