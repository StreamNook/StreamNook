// Preconfigured image hosts for paste-to-upload (the ShareX idea: pick a
// destination once, then every paste just works). Each preset is the exact
// multipart request the host documents; Rust does the POST. Hosts here are
// also on the link-preview trust list, so the returned link renders as an
// image card in chat for everyone using StreamNook.
//
// Each preset was proven live with the app's exact multipart request on
// 2026-09-07 (catbox, uguu, nuuls answered with a link). 0x0.st was dropped:
// its uploads are switched off indefinitely.

import type { ImageUploaderSettings } from '../types';

export interface UploadHostPreset {
  id: string;
  label: string;
  /** One line a person can decide on: who runs it, how long files live. */
  note: string;
  url: string;
  formField: string;
  /** Extra form fields the host requires, sent as text parts. */
  extraFields?: Record<string, string>;
  /** Dotted path into a JSON reply; empty = reply body is the link. */
  responsePath?: string;
  /** Max upload the host accepts, for the hint. */
  maxMb: number;
}

export const UPLOAD_HOST_PRESETS: UploadHostPreset[] = [
  {
    id: 'nuuls',
    label: 'i.nuuls.com',
    note: "A long-running community image host. Public links, kept indefinitely.",
    url: 'https://i.nuuls.com/upload',
    formField: 'attachment',
    maxMb: 10,
  },
  {
    id: 'catbox',
    label: 'catbox.moe',
    note: 'Popular, permanent, no account. Public links.',
    url: 'https://catbox.moe/user/api.php',
    formField: 'fileToUpload',
    extraFields: { reqtype: 'fileupload' },
    maxMb: 200,
  },
  {
    id: 'litterbox',
    label: 'Litterbox (72 h)',
    note: "catbox's temporary sibling: the link dies after 72 hours.",
    url: 'https://litterbox.catbox.moe/resources/internals/api.php',
    formField: 'fileToUpload',
    extraFields: { reqtype: 'fileupload', time: '72h' },
    maxMb: 1000,
  },
  {
    id: 'uguu',
    label: 'uguu.se (3 h)',
    note: 'Throwaway host: files vanish after about three hours.',
    url: 'https://uguu.se/upload?output=text',
    formField: 'files[]',
    maxMb: 128,
  },
];

export const CUSTOM_HOST_ID = 'custom';
export const DEFAULT_HOST_ID = 'nuuls';

export interface ResolvedUploadTarget {
  url: string;
  formField: string;
  extraFields: Record<string, string>;
  responsePath: string;
  label: string;
}

/** The request the composer should make for the current settings. */
export function resolveUploadTarget(up: ImageUploaderSettings | undefined): ResolvedUploadTarget {
  const presetId = up?.preset ?? (up?.url && up.url !== UPLOAD_HOST_PRESETS[0].url ? CUSTOM_HOST_ID : DEFAULT_HOST_ID);
  const preset = UPLOAD_HOST_PRESETS.find((p) => p.id === presetId);
  if (preset) {
    return {
      url: preset.url,
      formField: preset.formField,
      extraFields: preset.extraFields ?? {},
      responsePath: preset.responsePath ?? '',
      label: preset.label,
    };
  }
  return {
    url: up?.url ?? '',
    formField: up?.form_field || 'file',
    extraFields: parseExtraFields(up?.extra_fields),
    responsePath: up?.response_path ?? '',
    label: 'your host',
  };
}

/** "reqtype=fileupload&time=72h" -> { reqtype: "fileupload", time: "72h" } */
export function parseExtraFields(raw: string | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  if (!raw) return out;
  for (const pair of raw.split('&')) {
    const i = pair.indexOf('=');
    if (i <= 0) continue;
    const k = decodeURIComponent(pair.slice(0, i).trim());
    const v = decodeURIComponent(pair.slice(i + 1).trim());
    if (k) out[k] = v;
  }
  return out;
}

export function encodeExtraFields(fields: Record<string, string>): string {
  return Object.entries(fields)
    .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`)
    .join('&');
}
