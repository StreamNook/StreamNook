// Writes the iPhone emoji picker's catalog (ios/Sources/SNKit/Resources/emoji.json) from the
// desktop's emoji data, so both pickers list the same emoji in the same categories.
//
//   node ios/scripts/emoji-catalog.mjs
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const source = readFileSync(join(repo, 'src/services/emojiCategories.ts'), 'utf8');

// The file is two object literals of string arrays; strip the types and read them as JSON5-ish
// JavaScript in a sandboxed function.
const body = source
  .replace(/export const (\w+): Record<string, string\[\]> =/g, 'out.$1 =')
  .replace(/export /g, '');
const out = {};
new Function('out', body)(out);

const catalog = {
  categories: Object.entries(out.EMOJI_CATEGORIES).map(([name, emoji]) => ({ name, emoji })),
  keywords: out.EMOJI_KEYWORDS,
};
const target = join(repo, 'ios/Sources/SNKit/Resources/emoji.json');
writeFileSync(target, JSON.stringify(catalog));
console.log(`wrote ${catalog.categories.length} categories to ${target}`);
