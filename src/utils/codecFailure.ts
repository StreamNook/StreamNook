import Hls from 'hls.js';

/** hls.js errors meaning this machine cannot play the stream's codec at all. */
export const UNSUPPORTED_CODEC = new Set<string>([
  Hls.ErrorDetails.BUFFER_ADD_CODEC_ERROR,
  Hls.ErrorDetails.BUFFER_INCOMPATIBLE_CODECS_ERROR,
  Hls.ErrorDetails.MANIFEST_INCOMPATIBLE_CODECS_ERROR,
]);

/**
 * Whether a fatal media error means the codec cannot play here: the
 * unsupported-codec errors only. A decode error is a bad segment as often as a
 * bad codec, so it takes the normal media recovery on the same codec and never
 * steps away from it. The verdict reads nothing about the player's past, so a
 * reload cannot change it.
 */
export function isCodecFailure(details: string): boolean {
  return UNSUPPORTED_CODEC.has(details);
}
