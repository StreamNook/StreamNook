// First-load placeholders shaped like MobileStreamCard: a quiet pulse on the
// surface tokens, no shimmer gradients.
import React from 'react';

export const SkeletonCards: React.FC<{ count?: number }> = ({ count = 3 }) => (
  <div className="flex flex-col gap-3 px-4 pb-4">
    {Array.from({ length: count }, (_, i) => (
      <div key={i} className="glass-panel overflow-hidden animate-pulse">
        <div className="w-full aspect-video bg-surface" />
        <div className="px-3 py-2.5">
          <div className="h-4 w-3/4 rounded bg-surface mb-2" />
          <div className="h-3 w-1/3 rounded bg-surface" />
        </div>
      </div>
    ))}
  </div>
);
