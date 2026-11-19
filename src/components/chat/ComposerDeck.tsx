// The composer's deck and teaching placeholder as small leaves of the message
// box, so the hint, the GIF-link probe and the chat's emote of the moment
// re-render these and nothing else (chat's composer is a very large
// component). The composer says when the box may teach (`HintGate`); these
// pick the hint and the face and draw the hint layer over the empty field.
import type { CSSProperties, Ref } from 'react';
import { useEmoteMoment } from '../../hooks/useEmoteMoment';
import type { ProviderId } from '../../types/providers';
import { ComposerHintLayer } from './ComposerHintLayer';
import {
  deckFace,
  fanCards,
  hintCycle,
  HINT_WORDS,
  useComposerHint,
  useGifLinkInBox,
  type DeckCard,
  type HintGate,
} from './composerHints';
import { DeckFace, EmoteDeckButton } from './EmoteDeckButton';

export interface DeckRoom {
  provider: ProviderId;
  /** The channel as the composer names it to Rust's emote matching. */
  channel: string;
}

const roomKeyOf = (room: DeckRoom | null) => (room ? `${room.provider}:${room.channel}` : '');

/** The button, with the chat's moment asked for only while it shows. */
function DeckButton({
  room,
  face,
  cards,
  ...rest
}: {
  room: DeckRoom | null;
  face: DeckCard;
  cards: readonly DeckCard[];
  open: boolean;
  smiley: string;
  label: string;
  buttonRef?: Ref<HTMLButtonElement>;
  onClick?: () => void;
  onMouseLeave?: () => void;
}) {
  const moment = useEmoteMoment(room?.provider ?? null, room?.channel ?? null);
  return <EmoteDeckButton face={face} cards={cards} moment={moment} {...rest} />;
}

/** Chat's composer: the deck inset at the field's left, and the hints over it. */
export function ComposerDeck({
  showButton,
  open,
  room,
  smiley,
  label,
  slash,
  gifs,
  text,
  gate,
  layerClassName,
  layerStyle,
  buttonRef,
  onClick,
  onMouseLeave,
}: {
  showButton: boolean;
  open: boolean;
  room: DeckRoom | null;
  smiley: string;
  label: string;
  /** The menu offers commands: the slash card and its hint. */
  slash: boolean;
  /** The menu offers GIFs here: the GIF chip and its hint. */
  gifs: boolean;
  text: string;
  gate: HintGate;
  layerClassName?: string;
  layerStyle?: CSSProperties;
  buttonRef?: Ref<HTMLButtonElement>;
  onClick?: () => void;
  onMouseLeave?: () => void;
}) {
  // The GIF hint points at the button's menu: without the button it is only
  // the prompt and commands.
  const cycle = hintCycle({ slash, gifs: gifs && showButton });
  const hint = useComposerHint(gate.active, cycle, roomKeyOf(room), text);
  // Only a shown button can show a GIF chip.
  const gifLink = useGifLinkInBox(text, gifs && showButton);
  const face = deckFace({ text, gifLink, slash, gifs });
  return (
    <>
      {showButton && (
        <DeckButton
          room={room}
          open={open}
          face={face}
          cards={fanCards(face, { slash, gifs })}
          smiley={smiley}
          label={label}
          buttonRef={buttonRef}
          onClick={onClick}
          onMouseLeave={onMouseLeave}
        />
      )}
      {gate.layer && (
        <ComposerHintLayer
          hintKey={hint}
          wide={HINT_WORDS[hint].wide}
          narrow={HINT_WORDS[hint].narrow}
          className={layerClassName}
          style={layerStyle}
        />
      )}
    </>
  );
}

/** The phone's face: the chat's emote of the moment, or the GIF chip while
 *  the box holds a GIF link. No hover there, so no fan. */
export function RoomDeckFace({
  room,
  text,
  gifs,
  smiley,
}: {
  room: DeckRoom | null;
  text: string;
  gifs: boolean;
  smiley: string;
}) {
  const moment = useEmoteMoment(room?.provider ?? null, room?.channel ?? null);
  const gifLink = useGifLinkInBox(text, gifs);
  return <DeckFace face={deckFace({ text, gifLink, slash: false, gifs })} moment={moment} smiley={smiley} size="phone" />;
}

/** The phone's hint layer; `pick` chooses the wording that fits the field. */
export function PhoneHintLayer({
  room,
  text,
  gifs,
  gate,
  pick,
  className,
}: {
  room: DeckRoom | null;
  text: string;
  gifs: boolean;
  gate: HintGate;
  pick: (wordings: string[]) => string;
  className?: string;
}) {
  const hint = useComposerHint(gate.active, hintCycle({ slash: false, gifs }), roomKeyOf(room), text);
  if (!gate.layer) return null;
  return <ComposerHintLayer hintKey={hint} wide={pick([HINT_WORDS[hint].wide, HINT_WORDS[hint].narrow])} className={className} />;
}
