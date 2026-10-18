/**
 * Stream languages for Discover and Kick browse (`discovery_languages`). Rust
 * reads the setting (services/browse_languages.rs): empty follows the device
 * language, `any` shows every language, anything else is the list picked.
 *
 * Twitch broadcast languages selectable in the Discover feed filter, in Helix
 * code form ("fr", "zh-hk"). Labels are native names, matching Twitch's own
 * directory language picker. ASL is deliberately absent: streams tagged ASL
 * report a spoken language (usually "en") in the language field, so an ASL
 * selection could never match.
 */
export interface DiscoveryLanguage {
  value: string;
  label: string;
}

export const DISCOVERY_LANGUAGES: DiscoveryLanguage[] = [
  { value: 'en', label: 'English' },
  { value: 'es', label: 'Español' },
  { value: 'fr', label: 'Français' },
  { value: 'de', label: 'Deutsch' },
  { value: 'it', label: 'Italiano' },
  { value: 'pt', label: 'Português' },
  { value: 'ru', label: 'Русский' },
  { value: 'ja', label: '日本語' },
  { value: 'ko', label: '한국어' },
  { value: 'zh', label: '中文' },
  { value: 'zh-hk', label: '粵語' },
  { value: 'ar', label: 'العربية' },
  { value: 'tr', label: 'Türkçe' },
  { value: 'pl', label: 'Polski' },
  { value: 'nl', label: 'Nederlands' },
  { value: 'sv', label: 'Svenska' },
  { value: 'no', label: 'Norsk' },
  { value: 'da', label: 'Dansk' },
  { value: 'fi', label: 'Suomi' },
  { value: 'cs', label: 'Čeština' },
  { value: 'sk', label: 'Slovenčina' },
  { value: 'hu', label: 'Magyar' },
  { value: 'ro', label: 'Română' },
  { value: 'bg', label: 'Български' },
  { value: 'el', label: 'Ελληνικά' },
  { value: 'th', label: 'ไทย' },
  { value: 'vi', label: 'Tiếng Việt' },
  { value: 'id', label: 'Bahasa Indonesia' },
  { value: 'ms', label: 'Bahasa Melayu' },
  { value: 'hi', label: 'हिन्दी' },
  { value: 'tl', label: 'Filipino' },
  { value: 'uk', label: 'Українська' },
  { value: 'ca', label: 'Català' },
  { value: 'other', label: 'Other' },
];

/** The pick that turns the filter off. */
export const ANY_LANGUAGE = 'any';

/** Every choice the Languages row offers: "Any language" first. */
export const DISCOVERY_LANGUAGE_OPTIONS: DiscoveryLanguage[] = [
  { value: ANY_LANGUAGE, label: 'Any language' },
  ...DISCOVERY_LANGUAGES,
];

/** The empty setting's label: it follows the device language. */
export const DEVICE_LANGUAGE_LABEL = 'Device language';

/**
 * The value to save when the picks change from `prev` to `next`. "Any
 * language" stands alone: picking it clears the languages, and picking a
 * language while it is on drops it.
 */
export function nextDiscoveryLanguages(prev: string[], next: string[]): string[] {
  const hadAny = prev.includes(ANY_LANGUAGE);
  const hasAny = next.includes(ANY_LANGUAGE);
  if (hasAny && !hadAny) return [ANY_LANGUAGE];
  if (hasAny && next.length > 1) return next.filter((l) => l !== ANY_LANGUAGE);
  return next;
}
