import { useEffect } from 'react';
import { useAppStore } from '../../stores/AppStore';
import {
  isAnimatedPicture,
  primeSevenTvPicture,
  type ProfilePictureAnimation,
  type ProfilePictureSource,
  type SevenTvPicture,
} from '../../services/sevenTvPictures';
import { useProfilePicture } from '../../hooks/useProfilePicture';
import { SevenTvAvatar } from '../SevenTvAvatar';
import { SettingPreview } from './ChatPreview';
import sampleStill from '../../assets/streamnook-badge-gold-128.webp';
import sampleMoving64 from '../../assets/streamnook-badge-gold-animated-64.webp';
import sampleMoving128 from '../../assets/streamnook-badge-gold-animated.webp';
import samplePlatform from '../../assets/streamnook-logo-128.webp';

// The "Profile pictures" source and its "Animation" choice, drawn with the
// real avatar on the two kinds of surface the animation treats differently: a
// list row (Sidebar, Home, whispers) and a focal card (profile card, pins, the
// channel header). The viewer's own pictures are used when their 7TV one
// moves; otherwise a sample whose platform picture is the StreamNook logo and
// whose 7TV picture is the moving gold badge.

const SAMPLE_ID = 'streamnook:profile-picture-sample';
const SAMPLE: SevenTvPicture = {
  id: SAMPLE_ID,
  still: [{ url: sampleStill, scale: 4 }],
  animated: [
    { url: sampleMoving64, scale: 2 },
    { url: sampleMoving128, scale: 4 },
  ],
};

const CAPTIONS: Record<ProfilePictureAnimation, string> = {
  always: 'every picture moves, lists included',
  hover: 'cards move; a list picture moves while you point at its row',
  never: 'every picture stays still',
};

const PLATFORM_CAPTION = 'everyone shows their own platform picture';

export const ProfilePicturePreview = ({
  animation,
  source,
}: {
  animation: ProfilePictureAnimation;
  source: ProfilePictureSource;
}) => {
  const me = useAppStore((s) => s.currentUser);
  const own = useProfilePicture(me?.user_id ?? null);

  useEffect(() => {
    primeSevenTvPicture(SAMPLE_ID, SAMPLE);
  }, []);

  const mine = isAnimatedPicture(own) && !!me?.user_id;
  const id = mine ? me.user_id : SAMPLE_ID;
  const fallback = mine ? me.profile_image_url : samplePlatform;
  const name = mine ? me.display_name || me.username : 'StreamNook';

  return (
    <SettingPreview caption={source === 'twitch' ? PLATFORM_CAPTION : CAPTIONS[animation]}>
      <div className="glass-panel flex items-center gap-6 rounded-lg px-4 py-3">
        <div data-avatar-hover="" className="flex min-w-0 flex-1 items-center gap-2.5 rounded-md px-2 py-1.5 transition-colors hover:bg-white/5">
          <SevenTvAvatar
            key={`list-${id}`}
            cosmeticsId={id}
            ask={mine}
            px={28}
            motion="hover"
            hoverScope="[data-avatar-hover]"
            animation={animation}
            source={source}
            src={fallback}
            alt=""
            className="h-7 w-7 shrink-0 rounded-full bg-white/5 object-cover ring-1 ring-borderSubtle"
          />
          <div className="min-w-0">
            <div className="truncate text-[13px] font-medium text-textPrimary">{name}</div>
            <div className="text-[11px] text-textMuted">In a list</div>
          </div>
        </div>
        <div className="flex shrink-0 flex-col items-center gap-1.5">
          <SevenTvAvatar
            key={`card-${id}`}
            cosmeticsId={id}
            ask={mine}
            px={48}
            motion="animate"
            animation={animation}
            source={source}
            src={fallback}
            alt=""
            className="h-12 w-12 rounded-full bg-white/5 object-cover ring-1 ring-borderSubtle"
          />
          <div className="text-[11px] text-textMuted">On a profile card</div>
        </div>
      </div>
    </SettingPreview>
  );
};
