// Stands in the player's band while a Cast device plays the stream: the phone
// is the remote here, and chat carries on below it as normal.
//
// Presentation only. Everything it shows is Rust's cast state, and every
// button is a command back to Rust (services/cast_session.rs).
import React from 'react';
import { DeviceMobile, Pause, Play, Screencast, Stop, Television, SpeakerSimpleHigh } from 'phosphor-react';
import { Logger } from '../../utils/logger';
import { castControl, type CastState } from './castState';
import { setTvScreenEnabled, tvControl, type TvScreenState } from './tvScreenState';

function statusLine(cast: CastState): string {
  if (cast.error) return cast.error;
  if (!cast.target) return 'Pick a stream to play it on the TV.';
  switch (cast.player) {
    case 'loading':
      return 'Starting the stream…';
    case 'buffering':
      return 'Buffering…';
    case 'paused':
      return 'Paused';
    case 'playing':
      return cast.target.title ? `${cast.target.name}: ${cast.target.title}` : cast.target.name;
    default:
      return 'Waiting for the TV…';
  }
}

export const CastCard: React.FC<{ cast: CastState; compact?: boolean }> = ({ cast, compact = false }) => {
  const device = cast.device ?? (cast.audioOnly ? 'the speaker' : 'the TV');
  const canToggle = cast.player === 'playing' || cast.player === 'paused';
  const paused = cast.player === 'paused';
  const Icon = cast.audioOnly ? SpeakerSimpleHigh : Television;

  const send = (action: 'play' | 'pause' | 'stop') =>
    castControl(action).catch((e) => Logger.warn(`[Cast] ${action}:`, e));

  if (compact) {
    // Mini box / PiP: just say where the stream went.
    return (
      <div className="w-full h-full bg-black flex items-center justify-center text-white/80">
        <Icon size={28} weight="fill" />
      </div>
    );
  }

  return (
    <div className="relative w-full h-full bg-black overflow-hidden flex flex-col items-center justify-center gap-3 px-6 text-center">
      <span className="w-12 h-12 rounded-full bg-accent/20 text-accent flex items-center justify-center">
        <Icon size={24} weight="fill" />
      </span>
      <div className="min-w-0 max-w-full">
        <div className="text-[15px] font-semibold text-white truncate">
          Casting to {device}
        </div>
        <div className={`text-[12.5px] leading-snug truncate ${cast.error ? 'text-error' : 'text-white/70'}`}>
          {statusLine(cast)}
        </div>
      </div>
      <div className="flex items-center gap-2">
        {canToggle && (
          <button
            onClick={() => void send(paused ? 'play' : 'pause')}
            className="chrome-glaze chrome-glaze--control chrome-glaze--frosted w-11 h-11 flex items-center justify-center text-white"
            aria-label={paused ? 'Play on the TV' : 'Pause on the TV'}
          >
            {paused ? <Play size={20} weight="fill" /> : <Pause size={20} weight="fill" />}
          </button>
        )}
        <button
          onClick={() => void send('stop')}
          className="chrome-glaze chrome-glaze--control chrome-glaze--frosted h-11 px-4 flex items-center gap-2 text-sm font-semibold text-white"
        >
          <Stop size={16} weight="fill" />
          Stop casting
        </button>
      </div>
    </div>
  );
};

/**
 * After a cast ends the phone's player is not restarted on its own: the session
 * may have ended with the phone in a pocket, and sound out of nowhere is worse
 * than a tap. This offers the tap.
 */
export const CastEndedCard: React.FC<{ onWatchHere: () => void; compact?: boolean }> = ({
  onWatchHere,
  compact = false,
}) => (
  <div className="w-full h-full bg-black flex flex-col items-center justify-center gap-3 px-6 text-center">
    {!compact && <div className="text-[13.5px] text-white/75">Casting ended</div>}
    <button
      onClick={(e) => {
        e.stopPropagation();
        onWatchHere();
      }}
      className="chrome-glaze chrome-glaze--control chrome-glaze--frosted h-11 px-5 flex items-center gap-2 text-sm font-semibold text-white"
    >
      <Play size={16} weight="fill" />
      Watch on this phone
    </button>
  </div>
);

/**
 * TV screen mode: the stream is playing full-frame on the mirrored TV, so the
 * phone's band shows where it went and the controls, and chat carries on below.
 * "Show my phone" hands the TV back to plain mirroring.
 */
export const TvCard: React.FC<{ tv: TvScreenState; compact?: boolean }> = ({ tv, compact = false }) => {
  const name = tv.display?.name || 'the TV';
  const paused = tv.player === 'paused';
  const canToggle = tv.player === 'playing' || tv.player === 'paused';
  const status = tv.error
    ? tv.error
    : tv.player === 'loading' || tv.player === 'buffering'
      ? 'Starting the stream…'
      : paused
        ? 'Paused'
        : 'Filling the TV while you mirror';

  if (compact) {
    return (
      <div className="w-full h-full bg-black flex items-center justify-center text-white/80">
        <Screencast size={28} weight="fill" />
      </div>
    );
  }

  return (
    <div className="relative w-full h-full bg-black overflow-hidden flex flex-col items-center justify-center gap-3 px-6 text-center">
      <span className="w-12 h-12 rounded-full bg-accent/20 text-accent flex items-center justify-center">
        <Screencast size={24} weight="fill" />
      </span>
      <div className="min-w-0 max-w-full">
        <div className="text-[15px] font-semibold text-white truncate">Playing on {name}</div>
        <div className={`text-[12.5px] leading-snug truncate ${tv.error ? 'text-error' : 'text-white/70'}`}>
          {status}
        </div>
      </div>
      <div className="flex items-center gap-2">
        {canToggle && (
          <button
            onClick={() => void tvControl(paused ? 'play' : 'pause').catch((e) => Logger.warn('[TvScreen] control:', e))}
            className="chrome-glaze chrome-glaze--control chrome-glaze--frosted w-11 h-11 flex items-center justify-center text-white"
            aria-label={paused ? 'Play on the TV' : 'Pause on the TV'}
          >
            {paused ? <Play size={20} weight="fill" /> : <Pause size={20} weight="fill" />}
          </button>
        )}
        <button
          onClick={() => void setTvScreenEnabled(false).catch((e) => Logger.warn('[TvScreen] off:', e))}
          className="chrome-glaze chrome-glaze--control chrome-glaze--frosted h-11 px-4 flex items-center gap-2 text-sm font-semibold text-white"
        >
          <DeviceMobile size={16} weight="fill" />
          Show my phone
        </button>
      </div>
    </div>
  );
};
