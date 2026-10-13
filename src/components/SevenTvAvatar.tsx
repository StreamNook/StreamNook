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
  isPictureRendition,
  pickSevenTvPictureUrl,
  pictureMovesNow,
  picturePlan,
  type ProfilePictureAnimation,
  type ProfilePictureSource,
  type SevenTvMotion,
} from '../services/sevenTvPictures';
import { useProfilePicture } from '../hooks/useProfilePicture';
import { isWindowHidden, onWindowVisibility } from '../utils/windowVisibility';
import { watchPictureVisibility } from '../utils/pictureVisibility';

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

// Renditions this window has drawn or is about to, held so a paused picture
// resumes without a blank frame: the swap waits for `decode()`, which is
// instant for one already held. Bounded; dropping one only costs a re-decode.
const HELD_MAX = 64;
const held = new Map<string, HTMLImageElement>();

function decodeRendition(url: string): Promise<void> {
  let img = held.get(url);
  if (img) {
    held.delete(url);
  } else {
    img = new Image();
    img.decoding = 'async';
    img.src = url;
  }
  held.set(url, img);
  if (held.size > HELD_MAX) {
    const oldest = held.keys().next().value;
    if (oldest !== undefined) held.delete(oldest);
  }
  return img.decode();
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
  // moving by Always draws its still until the watcher says it is in view,
  // so pictures scrolled out of sight never fetch their animation. In view
  // means seen: on screen, drawn, and not under an overlay or the full-screen
  // player (utils/pictureVisibility).
  const [inView, setInView] = useState(motion === 'animate');
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
  const tracksInView = animated && plan === 'moving';
  const tracksHover = animated && plan === 'hover' && hovered === undefined;
  const tracksScope = tracksHover && !!hoverScope;

  const moving = pictureMovesNow(plan, { hovered: hovered ?? ownHover, onScreen: inView, windowHidden });
  const sevenTvUrl = pickSevenTvPictureUrl(picture, {
    px,
    animate: moving,
    dpr: typeof window !== 'undefined' ? window.devicePixelRatio : 1,
  });
  const useSevenTv = !!sevenTvUrl && sevenTvUrl !== failedUrl;
  const wanted = (useSevenTv ? sevenTvUrl : src) || null;

  // Pausing and resuming swap between two renditions of one picture. That
  // swap waits until the next one is decoded, so the picture never blanks or
  // flickers; anything else (the platform picture, a different person) is
  // drawn at once.
  const [shown, setShown] = useState(wanted);
  const swapWaits = shown !== wanted && isPictureRendition(picture, shown) && isPictureRendition(picture, wanted);
  if (shown !== wanted && !swapWaits) setShown(wanted);
  useEffect(() => {
    if (!swapWaits || !wanted) return;
    let live = true;
    const show = () => {
      if (live) setShown(wanted);
    };
    // A failed decode still swaps, so the image's own error path decides.
    decodeRendition(wanted).then(show, show);
    return () => {
      live = false;
    };
  }, [swapWaits, wanted]);
  const showsSevenTv = isPictureRendition(picture, shown);

  useEffect(() => {
    const el = imgRef.current;
    if (!tracksInView || !el) return;
    return watchPictureVisibility(el, setInView);
  }, [tracksInView]);

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
      src={shown || undefined}
      data-seventv-picture={showsSevenTv ? '' : undefined}
      onError={(e) => {
        if (showsSevenTv) {
          setFailedUrl(shown);
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
