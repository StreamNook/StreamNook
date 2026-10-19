import Hls from 'hls.js';

/** hls.js errors meaning this machine cannot play the stream's codec at all. */
export const UNSUPPORTED_CODEC = new Set<string>([
  Hls.ErrorDetails.BUFFER_ADD_CODEC_ERROR,
  Hls.ErrorDetails.BUFFER_INCOMPATIBLE_CODECS_ERROR,
  Hls.ErrorDetails.MANIFEST_INCOMPATIBLE_CODECS_ERROR,
]);

/** `MediaError` codes, named here so this also runs where that global is absent. */
const MEDIA_ERR_DECODE = 3;
const MEDIA_ERR_SRC_NOT_SUPPORTED = 4;

/**
 * Whether a fatal media error is the codec failing on this machine: an
 * unsupported-codec error, or a decode failure before the first frame (after
 * one, a decode error is more likely a bad segment than the codec).
 */
export function isCodecFailure(
  details: string,
  video: Pick<HTMLVideoElement, 'error'>,
  everPlayed: boolean,
): boolean {
  if (UNSUPPORTED_CODEC.has(details)) return true;
  const code = video.error?.code;
  return !everPlayed && (code === MEDIA_ERR_DECODE || code === MEDIA_ERR_SRC_NOT_SUPPORTED);
}
