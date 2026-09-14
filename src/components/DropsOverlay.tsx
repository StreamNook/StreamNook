import { X, Gift } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import DropsCenter from './DropsCenter';
import { Tooltip } from './ui/Tooltip';
import { motion, AnimatePresence } from 'framer-motion';

export default function DropsOverlay() {
  // Action without a subscription; only the visibility flag drives re-renders.
  const { setShowDropsOverlay } = useAppStore.getState();
  const showDropsOverlay = useAppStore((s) => s.showDropsOverlay);

  return (
    <AnimatePresence>
      {showDropsOverlay && (
        <motion.div 
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.15 }}
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-2xl"
        >
          {/* Hover-sensitive background overlay */}
          <div
            className="absolute inset-0 group-hover:pointer-events-none"
            onClick={() => setShowDropsOverlay(false)}
          />

          <motion.div 
            initial={{ opacity: 0, scale: 0.95, y: 15 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.95, y: 15 }}
            transition={{ type: "spring", stiffness: 350, damping: 25 }}
            style={{ willChange: "transform, opacity" }}
            className="w-[95vw] max-w-[1800px] h-[90vh] liquid-glass-panel flex flex-col overflow-hidden relative z-10"
          >
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-borderSubtle">
          <h2 className="text-lg font-semibold text-textPrimary flex items-center gap-2">
            <Gift size={20} className="text-accent" />
            Drops & Channel Points
          </h2>
          <Tooltip content="Close" delay={200} side="left">
            <button
              onClick={() => setShowDropsOverlay(false)}
              className="p-1.5 text-textSecondary hover:text-textPrimary hover:bg-glass rounded transition-all duration-200"
            >
              <X size={18} />
            </button>
          </Tooltip>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-hidden">
          <DropsCenter />
        </div>
        </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
