// The main chat header's two-sided switch while a chat is pinned: the pinned
// chat and the stream being watched. It takes the title's place rather than a
// row of its own, and uses the header's segmented-control look and density
// steps. Each side is its streamer's picture, the pinned one wearing a small
// pin. Only the side on screen adds its name, cut short (6.5rem) so a long name
// never crowds the header; the other side is named by its tooltip. The name
// keeps a floor of width, so when the row runs short the header's density
// steps drop the uptime and the viewer capsule before the name of the chat you
// are typing into.
import { motion } from 'framer-motion';
import { PushPin } from 'phosphor-react';
import { Tooltip } from '../ui/Tooltip';
import { useAppStore } from '../../stores/AppStore';
import { showChatPinSide, type ChatPinView } from '../../stores/chatPinStore';
import { usePinPair } from '../../hooks/useChatPin';

export default function ChatPinSwitch() {
  const { pin, live, split, showPinned } = usePinPair();
  const liveName = useAppStore((s) => s.currentStream?.user_name || s.currentStream?.user_login || '');
  const liveAvatar = useAppStore((s) => s.currentStream?.profile_image_url || null);
  if (!split || !pin || !live) return null;

  const flip = (to: ChatPinView) => {
    if ((to === 'pinned') === showPinned) return;
    void showChatPinSide(to);
  };

  const sides = [
    { key: 'pinned' as const, name: pin.display_name || pin.login, avatar: pin.avatar_url, active: showPinned, hint: 'Pinned chat' },
    { key: 'live' as const, name: liveName || live.login, avatar: liveAvatar, active: !showPinned, hint: 'Chat of the stream you are watching' },
  ];

  return (
    <div
      // Shrinks with its wrapper, whose min width is this switch's floor (two
      // pictures and the name's 3.5rem); the name truncates down to that.
      className="chrome-glaze chrome-glaze--flat chat-header-capsule pointer-events-auto min-w-0 max-w-full"
    >
      {sides.map((side) => (
        <Tooltip key={side.key} content={`${side.hint}: ${side.name}`} side="bottom">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              flip(side.key);
            }}
            aria-label={`${side.hint}: ${side.name}`}
            aria-pressed={side.active}
            className={`chat-header-pill transition-colors ${side.active ? 'min-w-0 !pl-0.5 !pr-2.5 text-textPrimary' : 'flex-shrink-0 !px-0.5 text-textSecondary opacity-80 hover:text-textPrimary hover:opacity-100'}`}
          >
            {side.active && (
              <motion.span
                layoutId="chat-pin-pill"
                className="absolute inset-0 rounded-full bg-white/[0.13]"
                style={{ boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.09)' }}
                transition={{ type: 'spring', stiffness: 480, damping: 28 }}
              />
            )}
            <span className="relative z-10 grid h-[18px] w-[18px] flex-shrink-0 place-items-center">
              <span className="grid h-[18px] w-[18px] place-items-center overflow-hidden rounded-full bg-white/15 text-[9px] font-bold text-white">
                {side.avatar ? (
                  <img
                    src={side.avatar}
                    alt=""
                    draggable={false}
                    className="h-full w-full object-cover"
                    onError={(e) => {
                      (e.currentTarget as HTMLImageElement).style.display = 'none';
                    }}
                  />
                ) : (
                  side.name.charAt(0).toUpperCase()
                )}
              </span>
              {side.key === 'pinned' && (
                <span className="absolute -bottom-1 -right-1 grid h-[11px] w-[11px] place-items-center rounded-full bg-accent text-background">
                  <PushPin size={7} weight="fill" aria-hidden />
                </span>
              )}
            </span>
            {side.active && (
              <span data-fit-label className="chat-pin-name relative z-10 min-w-[3.5rem] max-w-[6.5rem] truncate">
                {side.name}
              </span>
            )}
          </button>
        </Tooltip>
      ))}
    </div>
  );
}
