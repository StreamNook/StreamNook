// Adapters mapping the existing badge (`cosmetics`) and atmosphere catalogs into
// the unified CosmeticDefinition, so surfaces can read one shape without a schema
// change. The source catalogs are untouched; this is a pure read model.

import type { Atmosphere } from '../atmospheres';
import type { CosmeticCatalogEntry } from '../supabaseService';
import {
  SLOT_FOR_TYPE,
  asAssetManifest,
  type CosmeticDefinition,
  type CosmeticType,
} from './types';

// A `cosmetics` row carrying the not-yet-populated manifest column.
type BadgeRow = CosmeticCatalogEntry & { asset_manifest?: unknown };

// Types that a `cosmetics.kind` may already name; anything else falls back to a
// badge (the historical default for that table).
const KNOWN_TYPES: readonly CosmeticType[] = [
  'badge',
  'frame',
  'cipher',
  'member_plate',
  'relic',
  'accolade_pin_style',
];

// Map a badge-family `cosmetics` row. Atmosphere ownership-handle rows
// (kind 'atmosphere') are NOT definitions here; cosmeticFromAtmosphere owns those.
export function cosmeticFromBadgeRow(row: BadgeRow): CosmeticDefinition | null {
  if (row.kind === 'atmosphere') return null;
  const cosmeticType: CosmeticType = (KNOWN_TYPES as readonly string[]).includes(row.kind)
    ? (row.kind as CosmeticType)
    : 'badge';
  return {
    slug: row.slug,
    name: row.name,
    description: row.description ?? null,
    cosmeticType,
    slot: SLOT_FOR_TYPE[cosmeticType],
    rarity: null,
    sourceType: 'streamnook',
    eventSlug: null,
    entitlementRequirement: 'none',
    ownershipPolicy: 'permanent',
    renderTier: 'none',
    isAnimated: !!row.animated,
    supportsReducedMotion: !row.animated,
    releaseStatus: row.is_active ? 'released' : 'retired',
    signatureSetSlug: null,
    assetManifest: asAssetManifest(row.asset_manifest),
    isDefault: !!row.is_default,
    hidden: !!row.hidden,
    sortOrder: row.sort_order ?? 0,
    metadata: { assetPath: row.asset_path },
  };
}

// Map an atmosphere. Its unlock carries real gating: accolade atmospheres are
// free once earned; the rest are subscriber-tier. Cologne rows name their
// bespoke renderer so the exact look is preserved.
export function cosmeticFromAtmosphere(atm: Atmosphere): CosmeticDefinition {
  const accoladeId = atm.unlock?.kind === 'accolade' ? atm.unlock.accoladeId : null;
  const subscriberGated = !atm.unlock || atm.unlock.kind === 'subscriber';
  return {
    slug: atm.id,
    name: atm.name,
    description: null,
    cosmeticType: 'atmosphere',
    slot: 'atmosphere',
    rarity: null,
    sourceType: accoladeId ? 'achievement' : 'subscription',
    eventSlug: null,
    entitlementRequirement: subscriberGated ? 'subscriber' : 'none',
    ownershipPolicy: 'permanent',
    renderTier: 'none',
    // Atmospheres always carry motion (a CSS curtain or an animated image).
    isAnimated: true,
    supportsReducedMotion: true,
    releaseStatus: 'released',
    signatureSetSlug: null,
    // The manifest column is not on the client Atmosphere shape yet; carried
    // through once the registry reads it.
    assetManifest: null,
    isDefault: false,
    hidden: false,
    sortOrder: 0,
    metadata: {
      accent: atm.accent,
      swatch: atm.swatch,
      chatEdge: atm.chatEdge,
      chatFrost: atm.chatFrost ?? false,
      chatBlur: atm.chatBlur ?? 0,
      chatRim: atm.chatRim ?? null,
      renderer: atm.kind === 'cologne-chrome' ? 'cologne-chrome' : undefined,
      accoladeId,
    },
  };
}

// Map both live catalogs into one deduped definition list. An atmosphere wins
// its slug over any `cosmetics` ownership-handle of the same slug.
export function buildCosmeticDefinitions(
  badgeRows: readonly BadgeRow[],
  atmospheres: readonly Atmosphere[],
): CosmeticDefinition[] {
  const bySlug = new Map<string, CosmeticDefinition>();
  for (const atm of atmospheres) {
    const def = cosmeticFromAtmosphere(atm);
    bySlug.set(def.slug, def);
  }
  for (const row of badgeRows) {
    const def = cosmeticFromBadgeRow(row);
    if (def && !bySlug.has(def.slug)) bySlug.set(def.slug, def);
  }
  return [...bySlug.values()];
}
