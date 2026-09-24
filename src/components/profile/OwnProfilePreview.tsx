// A live preview of your own StreamNook profile while you edit it in Settings.
//
// It is the same member card anyone sees when they click your name in chat
// (UserProfileCard), fed the edits in progress (hidden sections, profile
// theme, badge loadout) so every change shows as you make it. It sits above
// Settings so you can keep editing while it is open.

import UserProfileCard from '../UserProfileCard';
import { useAppStore } from '../../stores/AppStore';

/** Must match UserProfileCard's MEMBER_CARD_WIDTH, to center the card. */
const MEMBER_CARD_WIDTH = 760;

const OwnProfilePreview = () => {
  const userId = useAppStore((s) => s.profileViewerUserId);
  const preview = useAppStore((s) => s.profileViewerPreview);
  const close = useAppStore((s) => s.closeProfileViewer);
  const me = useAppStore((s) => s.currentUser);

  if (!userId || !me || me.user_id !== userId) return null;
  // The card anchors to the LEFT of `position`; this centers it.
  const position = { x: Math.round((window.innerWidth + MEMBER_CARD_WIDTH) / 2) + 10, y: 48 };
  return (
    <div className="relative z-[60]">
      <UserProfileCard
        userId={me.user_id}
        username={me.login ?? me.username}
        displayName={me.display_name ?? me.login ?? me.username}
        color="#9146FF"
        badges={[]}
        messageHistory={[]}
        onClose={close}
        position={position}
        profilePreview={preview}
      />
    </div>
  );
};

export default OwnProfilePreview;
