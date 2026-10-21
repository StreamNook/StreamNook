// Holds a subtree still while `hold` stays true: the chat list under a mini
// player, in PiP, in a chat-less landscape, or with the app backgrounded. The
// pane keeps its room and its sheets; the list just stops re-rendering, and on
// wake renders once with whatever arrived.
import React, { memo } from 'react';
import { holdStill } from './holdStill';

export const HoldWhile = memo(function HoldWhile({ children }: { hold: boolean; children: React.ReactNode }) {
  return <>{children}</>;
}, holdStill);
