import {
  forwardRef,
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ImgHTMLAttributes,
} from 'react';
import { useAppStore } from '../stores/AppStore';
import { isAnimatedPicture, pickSevenTvPictureUrl, useSevenTvPicture } from '../services/sevenTvPictures';

/**
 * How much a surface may move:
 * - `animate`: the moving picture (one person, a focal surface: profile card,
 *   pinned card, channel header);
 * - `still`: the first frame only (dense lists: chat rows, viewers);
 * - `hover`: the first frame until the pointer is over it (sidebar, tiles).
 */
export type SevenTvMotion = 'animate' | 'still' | 'hover';

export interface SevenTvAvatarProps extends Omit<ImgHTMLAttributes<HTMLImageElement>, 'src'> {
  /** The person's cosmetics id: a bare Twitch id, `kick:<id>` or `youtube:UC...`.
   *  Leave it out where the id would be from another key space. */
  cosmeticsId?: string | null;
  /** The platform's picture, drawn until a 7TV one is known, when there is
   *  none, when the setting is off, and when the 7TV image fails to load. */
  src?: string | null;
  /** The box this fills, in CSS px; picks the smallest sharp rendition. */
  px: number;
  motion?: SevenTvMotion;
  /** Hover state owned by a parent row, for `hover` motion. */
  hovered?: boolean;
  /** For `hover` motion: a selector for the ancestor whose hover moves the
   *  picture (a whole tile), for when something laid over the tile takes the
   *  pointer before the picture can. The picture listens to that element
   *  itself, so the tile re-renders nothing on hover. */
  hoverScope?: string;
  /** False where the person's cosmetics are already being resolved (chat
   *  rows): draw what that lookup primed and never ask on its own. */
  ask?: boolean;
}

const REDUCED_QUERY = '(prefers-reduced-motion: reduce)';
const reducedMotionQuery = typeof window !== 'undefined' ? window.matchMedia?.(REDUCED_QUERY) : undefined;
const subscribeReduced = (notify: () => void) => {
  reducedMotionQuery?.addEventListener?.('change', notify);
  return () => reducedMotionQuery?.removeEventListener?.('change', notify);
};
const osReducesMotion = () => reducedMotionQuery?.matches ?? false;

/**
 * A person's picture: their 7TV profile picture when they set one and the
 * setting is on, otherwise the platform picture. Always one `<img>`, so the
 * box the caller sized never changes; only its source does.
 */
export const SevenTvAvatar = forwardRef<HTMLImageElement, SevenTvAvatarProps>(function SevenTvAvatar(
  { cosmeticsId, src, px, motion = 'still', hovered, hoverScope, ask = true, onError, onMouseEnter, onMouseLeave, ...img },
  ref,
) {
  const enabled = useAppStore((s) => s.settings.cosmetics?.seventv_profile_pictures) ?? true;
  const motionMode = useAppStore((s) => s.settings.motion_mode) ?? 'full';
  const osReduced = useSyncExternalStore(subscribeReduced, osReducesMotion);
  const picture = useSevenTvPicture(enabled ? cosmeticsId : null, ask);
  const [ownHover, setOwnHover] = useState(false);
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const setRefs = useCallback(
    (el: HTMLImageElement | null) => {
      imgRef.current = el;
      if (typeof ref === 'function') ref(el);
      else if (ref) ref.current = el;
    },
    [ref],
  );

  const mayMove = motionMode === 'full' && !osReduced;
  const moving =
    mayMove && (motion === 'animate' || (motion === 'hover' && (hovered ?? ownHover)));
  const sevenTvUrl = pickSevenTvPictureUrl(picture, {
    px,
    animate: moving,
    dpr: typeof window !== 'undefined' ? window.devicePixelRatio : 1,
  });
  const useSevenTv = !!sevenTvUrl && sevenTvUrl !== failedUrl;
  // Nothing to listen for when the picture may not move (motion off or
  // reduced) or has no motion to show.
  const tracksHover = mayMove && motion === 'hover' && hovered === undefined && isAnimatedPicture(picture);
  const tracksScope = tracksHover && !!hoverScope;

  useEffect(() => {
    if (!tracksScope || !hoverScope) return;
    const scope = imgRef.current?.closest(hoverScope);
    if (!scope) return;
    const enter = () => setOwnHover(true);
    const leave = () => setOwnHover(false);
    scope.addEventListener('pointerenter', enter);
    scope.addEventListener('pointerleave', leave);
    // Already under the pointer when the picture became animated.
    if (scope.matches(':hover')) setOwnHover(true);
    return () => {
      scope.removeEventListener('pointerenter', enter);
      scope.removeEventListener('pointerleave', leave);
      setOwnHover(false);
    };
  }, [tracksScope, hoverScope]);

  return (
    <img
      ref={setRefs}
      decoding="async"
      {...img}
      src={(useSevenTv ? sevenTvUrl : src) || undefined}
      data-seventv-picture={useSevenTv ? '' : undefined}
      onError={(e) => {
        if (useSevenTv) {
          setFailedUrl(sevenTvUrl);
          return;
        }
        onError?.(e);
      }}
      onMouseEnter={(e) => {
        if (tracksHover && !tracksScope) setOwnHover(true);
        onMouseEnter?.(e);
      }}
      onMouseLeave={(e) => {
        if (tracksHover && !tracksScope) setOwnHover(false);
        onMouseLeave?.(e);
      }}
    />
  );
});
