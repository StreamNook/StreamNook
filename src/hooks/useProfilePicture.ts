// The viewer's "Profile pictures" setting applied to one person, for every
// surface that draws a face. `SevenTvAvatar` draws through it.
import { useAppStore } from '../stores/AppStore';
import {
  profilePictureSource,
  sevenTvLookupId,
  useSevenTvPicture,
  type ProfilePictureSource,
  type SevenTvPicture,
} from '../services/sevenTvPictures';

/** The saved source: 'seventv' (the default) or 'twitch'. */
export function useProfilePictureSource(): ProfilePictureSource {
  return profilePictureSource(useAppStore((s) => s.settings.cosmetics?.profile_picture_source));
}

/**
 * A person's 7TV picture under the viewer's source, or null. On the platform
 * source it asks for nothing, so no lookup reaches Rust or 7TV.
 * `source` overrides the saved choice (the settings preview).
 */
export function useProfilePicture(
  cosmeticsId: string | null | undefined,
  ask = true,
  source?: ProfilePictureSource,
): SevenTvPicture | null {
  const saved = useProfilePictureSource();
  return useSevenTvPicture(sevenTvLookupId(source ?? saved, cosmeticsId, ask), ask);
}
