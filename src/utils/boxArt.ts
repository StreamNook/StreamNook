// Category box art comes back from Twitch in two different shapes depending on
// which endpoint produced it, and only one of them is a template:
//
//   helix/games/top          -> ".../ttv-boxart/12345-{width}x{height}.jpg"
//   helix/search/categories  -> ".../ttv-boxart/12345-52x72.jpg"
//
// A plain `{width}` replace therefore no-ops on search results and leaves a
// 52x72 thumbnail to be stretched across a full card, which is why searched
// categories used to look badly blurred while browsed ones looked fine.
// Rewrite the baked-in size too.

const BAKED_SIZE = /-\d+x\d+\.(jpg|jpeg|png)$/i;

export function gameBoxArt(url: string | undefined, width = 1200, height = 1600): string {
  if (!url) return '';
  if (url.includes('{width}') && url.includes('{height}')) {
    return url.replace('{width}', String(width)).replace('{height}', String(height));
  }
  return url.replace(BAKED_SIZE, `-${width}x${height}.$1`);
}
