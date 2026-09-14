// Temporary stand-in while the real screens land phase by phase.
import React from 'react';

export const PlaceholderScreen: React.FC<{ title: string }> = ({ title }) => (
  <div className="sn-mobile-screen flex items-center justify-center">
    <div className="text-center px-8">
      <div className="text-base font-semibold text-textPrimary mb-1">{title}</div>
      <div className="text-sm text-textMuted">Coming soon on mobile.</div>
    </div>
  </div>
);
