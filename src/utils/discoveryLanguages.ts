/**
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
