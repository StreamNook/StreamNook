/** Geometry for the stacked "All platforms" mark.
 *
 *  The deck's total span is a property of the SLOT it sits in, not of how many
 *  platforms happen to be watchable: the step between cards shrinks as marks are
 *  added rather than the deck growing to fit them. A per-mark step works until
 *  it doesn't: at four marks the deck overhung its 12px flyout slot by 7px on
 *  each side, which is wider than the 10px gap to the label beside it.
 *
 *  The ratios are chosen so that at three marks this returns exactly the step it
 *  always had, which is what makes a three-platform build pixel-identical.
 */
export interface DeckGeometry {
  /** Side of one card. */
  card: number;
  /** Size the brand glyph is drawn at inside a card. */
  glyph: number;
  /** Horizontal step between consecutive cards. */
  dx: number;
  /** Vertical step between consecutive cards. */
  dy: number;
  /** Total width the deck occupies. */
  spread: number;
  /** Total height the deck occupies. */
  height: number;
}

/** Span of the deck as a multiple of one card, across and down. */
const SPAN_X = 1.88;
const SPAN_Y = 1.6;

export function deckGeometry(size: number, count: number): DeckGeometry {
  const card = Math.round(size * 0.88);
  const glyph = Math.round(card * 0.72);
  const n = Math.max(1, count);
  const dx = n > 1 ? Math.round((card * SPAN_X - card) / (n - 1)) : 0;
  const dy = n > 1 ? Math.round((card * SPAN_Y - card) / (n - 1)) : 0;
  return {
    card,
    glyph,
    dx,
    dy,
    spread: card + dx * (n - 1),
    height: card + dy * (n - 1),
  };
}
