// Emit small variants of the brand PNGs for badge/icon-sized render sites.
// The originals are 1019x1171 (~4.8MB decoded each) but render at 16-256px
// in chat rows, title bars and cards; full size stays for the About screen.
// Run: node scripts/resize-brand-assets.mjs
import sharp from 'sharp';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const assets = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'src', 'assets');

const jobs = [
  ['streamnook-logo.png', 'streamnook-logo-128.webp', 128],
  ['streamnook-logo.png', 'streamnook-logo-256.webp', 256],
  ['streamnook-badge-gold.png', 'streamnook-badge-gold-128.webp', 128],
];

for (const [src, out, size] of jobs) {
  const dest = path.join(assets, out);
  await sharp(path.join(assets, src))
    .resize({ height: size, withoutEnlargement: true })
    .webp({ quality: 92, effort: 6 })
    .toFile(dest);
  const meta = await sharp(dest).metadata();
  console.log(`${out}: ${meta.width}x${meta.height}`);
}
