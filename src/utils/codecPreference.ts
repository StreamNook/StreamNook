import { invoke } from '@tauri-apps/api/core';

/// Probe what this machine can actually decode and tell the Rust resolver which
/// video codecs it may select, most-preferred first. StreamNook resolves to a
/// SINGLE variant (there's no in-player codec fallback like a browser doing ABR
/// across the master), so resolving to a codec the GPU/OS can't decode would be a
/// black screen. We therefore only allow a codec the player reports as supported,
/// gated behind the user's `enhanced_codecs` setting, and always keep H.264 as the
/// universal fallback.
///
/// AV1 and HEVC variants also happen to ship as CMAF, so preferring them routes
/// low-latency channels through the LL-HLS origin (true ~2s) in addition to the
/// bandwidth saving the setting promises.
function canDecode(codec: string): boolean {
  try {
    return (
      typeof MediaSource !== 'undefined' &&
      MediaSource.isTypeSupported(`video/mp4; codecs="${codec}"`)
    );
  } catch {
    return false;
  }
}

/// The decoders every Twitch stream and VOD needs that this video engine lacks,
/// by name ("H.264", "AAC"). A Linux build made against the prebuilt CEF has
/// neither, so its player stays black and silent; empty everywhere else.
export function missingTwitchCodecs(
  supported: (type: string) => boolean = (type) =>
    typeof MediaSource !== 'undefined' && MediaSource.isTypeSupported(type),
): string[] {
  const can = (type: string) => {
    try {
      return supported(type);
    } catch {
      return false;
    }
  };
  const missing: string[] = [];
  if (!can('video/mp4; codecs="avc1.4d401f"') && !can('video/mp4; codecs="avc1.42e01e"')) missing.push('H.264');
  if (!can('audio/mp4; codecs="mp4a.40.2"')) missing.push('AAC');
  return missing;
}

/// The words the player shows once when `missingTwitchCodecs` is not empty.
export function missingCodecsWords(missing: string[]): string {
  const names = missing.join(' and ');
  return `This build's video engine can't decode ${names}, which Twitch video uses, so streams stay black. A build made from source needs a CEF built with proprietary codecs (see the README).`;
}

export function reportCodecPreference(enhancedCodecs: boolean): void {
  const prefs: string[] = [];
  if (enhancedCodecs) {
    // AV1: Chromium/WebView2 bundles a software decoder (dav1d), so it is almost
    // always playable. Probe a couple of common profiles/levels to be safe.
    if (canDecode('av01.0.08M.08') || canDecode('av01.0.05M.08')) {
      prefs.push('av1');
    }
    // HEVC/H.265: needs hardware support plus the Windows "HEVC Video Extensions";
    // frequently absent, which is exactly why it is probed rather than assumed.
    if (canDecode('hev1.1.6.L93.B0') || canDecode('hvc1.1.6.L93.B0')) {
      prefs.push('hevc');
    }
  }
  prefs.push('h264'); // universal fallback, always last-resort decodable
  invoke('set_codec_preference', { prefs }).catch(() => {
    /* non-critical: resolver keeps its current (H.264-safe) preference */
  });
}
