import { Fragment } from 'react';
import { nameSeams } from '../../utils/nameSeams';

/** A badge or cosmetic name for a narrow tile: it may wrap where the words of
 *  a run-together name meet ("PrincessDonut / Brown"). The tile's label also
 *  sets `[overflow-wrap:anywhere]`, so a single word wider than the tile still
 *  breaks inside it rather than spilling past the card. */
export const BreakableName = ({ name }: { name: string }) => (
  <>
    {nameSeams(name).map((piece, i) => (
      <Fragment key={i}>
        {i > 0 && <wbr />}
        {piece}
      </Fragment>
    ))}
  </>
);
