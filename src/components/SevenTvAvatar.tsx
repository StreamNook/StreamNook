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
import {
  isAnimatedPicture,
  pickSevenTvPictureUrl,
  pictureMovesNow,
  picturePlan,
  type ProfilePictureAnimation,
  type ProfilePictureSource,
  type SevenTvMotion,
} from '../services/sevenTvPictures';
import { useProfilePicture } from '../hooks/useProfilePicture';
import { isWindowHidden, onWindowVisibility } from '../utils/windowVisibility';

export type { SevenTvMotion } from '../services/sevenTvPictures';

export interface SevenTvAvatarProps extends Omit<ImgHTMLAttributes<HTMLImageElement>, 'src'> {
  /** The person's cosmetics id: a bare Twitch id, `kick:<id>` or `youtube:UC...`.
   *  Leave it out where the id would be from another key space. */
  cosmeticsId?: string | null;
  /** The platform's picture, drawn until a 7TV one is known, when there is
   *  none, on the platform source, and when the 7TV image fails to load. */
  src?: string | null;
  /** The box this fills, in CSS px; picks the smallest sharp rendition. */
  px: number;
  /** How much this surface moves under the viewer's "On hover" choice.
   *  Always (the default) and Never override it. */
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
  /** Draw as if the viewer had picked this choice (the settings preview). */
  animation?: ProfilePictureAnimation;
  /** Draw as if the viewer had picked this source (the settings preview). */
  source?: ProfilePictureSource;
}

const subscribeNothing = () => () => {};
const neverHidden = () => false;

// One IntersectionObserver for every moving picture in the window, so a list
// of a hundred costs one observer, not a hundred. A small margin starts the
// animation just before a picture scrolls in.
const onScreenListeners = new Map<Element, (onScreen: boolean) => void>();
let onScreenObserver: IntersectionObserver | null = null;

function watchOnScreen(el: Element, notify: (onScreen: boolean) => void): () => void {
  if (typeof IntersectionObserver === 'undefined') {
    notify(true);
    return () => {};
  }
  onScreenObserver ??= new IntersectionObserver(
    (entries) => {
      for (const entry of entries) onScreenListeners.get(entry.target)?.(entry.isIntersecting);
    },
    { rootMargin: '64px' },
  );
  onScreenListeners.set(el, notify);
  onScreenObserver.observe(el);
  return () => {
    onScreenObserver?.unobserve(el);
    onScreenListeners.delete(el);
  };
}

/**
 * A person's picture: their 7TV profile picture when they set one and the
 * viewer's source is 7TV, otherwise the platform picture. Always one `<img>`, so the
 * box the caller sized never changes; only its source does.
 */
export const SevenTvAvatar = forwardRef<HTMLImageElement, SevenTvAvatarProps>(function SevenTvAvatar(
  {
    cosmeticsId,
    src,
    px,
    motion = 'still',
    hovered,
    hoverScope,
    ask = true,
    animation,
    source,
    onError,
    onMouseEnter,
    onMouseLeave,
    ...img
  },
  ref,
) {
  const chosen = useAppStore((s) => s.settings.cosmetics?.profile_picture_animation);
  const picture = useProfilePicture(cosmeticsId, ask, source);
  const animated = isAnimatedPicture(picture);
  const plan = picturePlan(animation ?? chosen, motion);
  // A focal surface (`animate`) is in view when it mounts; a list upgraded to
  // moving by Always draws its still until the observer says it is in view,
  // so pictures scrolled out of sight never fetch their animation.
  const [onScreen, setOnScreen] = useState(motion === 'animate');
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

  // Only a picture that can move listens to the window, its scroll position
  // or the pointer; a still one listens to nothing.
  const tracksVisibility = animated && plan !== 'still';
  const windowHidden = useSyncExternalStore(
    tracksVisibility ? onWindowVisibility : subscribeNothing,
    tracksVisibility ? isWindowHidden : neverHidden,
  );
  const tracksOnScreen = animated && plan === 'moving';
  const tracksHover = animated && plan === 'hover' && hovered === undefined;
  const tracksScope = tracksHover && !!hoverScope;

  const moving = pictureMovesNow(plan, { hovered: hovered ?? ownHover, onScreen, windowHidden });
  const sevenTvUrl = pickSevenTvPictureUrl(picture, {
    px,
    animate: moving,
    dpr: typeof window !== 'undefined' ? window.devicePixelRatio : 1,
  });
  const useSevenTv = !!sevenTvUrl && sevenTvUrl !== failedUrl;

  useEffect(() => {
    const el = imgRef.current;
    if (!tracksOnScreen || !el) return;
    return watchOnScreen(el, setOnScreen);
  }, [tracksOnScreen]);

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
