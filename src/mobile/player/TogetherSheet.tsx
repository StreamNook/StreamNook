// Who the channel on screen is streaming with, as a bottom sheet: the desktop
// Together popover's panel, where tapping a member switches to their stream.
// The phone has no MultiNook grid, so the panel leaves those actions out.
import React from 'react';
import type { Collaboration } from '../../types';
import { useAppStore } from '../../stores/AppStore';
import { TogetherPanel } from '../../components/SharedViewers';
import { MobileSheet } from '../ui/MobileSheet';

export const TogetherSheet: React.FC<{ collab: Collaboration | null; open: boolean; onClose: () => void }> = ({
  collab,
  open,
  onClose,
}) => (
  <MobileSheet open={open && collab !== null} onClose={onClose}>
    {collab && (
      <TogetherPanel
        collab={collab}
        onOpenChannel={(login) => void useAppStore.getState().startStream(login)}
        onDone={onClose}
      />
    )}
  </MobileSheet>
);
