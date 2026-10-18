// The player's cast button opens this: the two ways onto a TV.
//
// Cast hands the stream itself to a Google Cast device (the TV fetches it and
// the phone becomes the remote). Mirroring is the system's own screen cast,
// which reaches the TVs and sticks Cast does not (Roku, Fire TV, most Samsung
// and LG sets over Miracast). Cast is listed only when this phone can cast at
// all (it needs Google Play services) and the stream is one a TV can fetch.
import React, { useState } from 'react';
import { CaretRight, Screencast, Television } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { Logger } from '../../utils/logger';
import { openCastPicker, openMirroring, useCastState } from './castState';
import { setTvScreenEnabled, useTvScreenState } from './tvScreenState';

const Row: React.FC<{
  icon: React.ReactNode;
  title: string;
  detail: string;
  onPress: () => void;
}> = ({ icon, title, detail, onPress }) => (
  <button
    onClick={onPress}
    className="flex items-center gap-3 w-full px-1 py-3 text-left active:bg-white/5 rounded-xl transition-colors"
  >
    <span className="w-10 h-10 shrink-0 rounded-full bg-accent/15 text-accent flex items-center justify-center">
      {icon}
    </span>
    <span className="flex-1 min-w-0">
      <span className="block text-[15px] font-semibold text-textPrimary">{title}</span>
      <span className="block text-[12.5px] text-textSecondary leading-snug">{detail}</span>
    </span>
    <CaretRight size={16} className="shrink-0 text-textMuted" />
  </button>
);

export const CastSheet: React.FC<{
  open: boolean;
  onClose: () => void;
  /** Whether the stream on screen is one a TV can fetch (Twitch, Kick). */
  castable: boolean;
}> = ({ open, onClose, castable }) => {
  const cast = useCastState();
  const tv = useTvScreenState();
  const [mirrorHint, setMirrorHint] = useState(false);

  const close = () => {
    setMirrorHint(false);
    onClose();
  };

  const startCast = () => {
    close();
    openCastPicker().catch((e) => Logger.warn('[Cast] picker:', e));
  };

  const startMirror = () => {
    openMirroring()
      .then((opened) => {
        if (opened) close();
        else setMirrorHint(true);
      })
      .catch(() => setMirrorHint(true));
  };

  return (
    <MobileSheet open={open} onClose={close} title="Watch on a TV">
      <div className="flex flex-col gap-1 pb-2">
        {cast.available && castable && (
          <Row
            icon={<Television size={20} weight="fill" />}
            title="Cast"
            detail="Plays on a TV or speaker with Google Cast. Your phone is the remote."
            onPress={startCast}
          />
        )}
        {tv.display && !tv.enabled && (
          <Row
            icon={<Screencast size={20} weight="fill" />}
            title={`Fill ${tv.display.name}`}
            detail="Plays the stream full-screen on the TV you're mirroring to. Your phone keeps chat."
            onPress={() => {
              close();
              void setTvScreenEnabled(true).catch((e) => Logger.warn('[TvScreen] on:', e));
            }}
          />
        )}
        <Row
          icon={<Screencast size={20} weight="fill" />}
          title="Mirror screen"
          detail="Shows your phone screen on any TV with screen mirroring."
          onPress={startMirror}
        />
        {mirrorHint && (
          <p className="px-1 pt-1 text-[12.5px] text-textSecondary leading-snug">
            This phone doesn&apos;t have a shortcut to mirroring. Swipe down from the top of the screen and look for
            Screen cast, Smart View or Screen mirroring in quick settings.
          </p>
        )}
        {!cast.available && (
          <p className="px-1 pt-1 text-[12.5px] text-textMuted leading-snug">
            Cast needs Google Play services, which this phone doesn&apos;t have. Mirroring still works.
          </p>
        )}
        {cast.available && !castable && (
          <p className="px-1 pt-1 text-[12.5px] text-textMuted leading-snug">
            Cast works with Twitch and Kick streams. Mirror your screen to watch this one on a TV.
          </p>
        )}
      </div>
    </MobileSheet>
  );
};
