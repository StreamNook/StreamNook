import { X, Store } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import PluginsSettings from './settings/PluginsSettings';
import { Tooltip } from './ui/Tooltip';
import { motion, AnimatePresence } from 'framer-motion';

/**
 * The plugin marketplace: a store for discovering, installing, and managing
 * plugins, opened from the title bar. It hosts the full plugin manager, which
 * lives here rather than in Settings.
 */
export default function MarketplaceOverlay() {
  // Action without a subscription; only the visibility flag drives re-renders.
  const { setShowMarketplaceOverlay } = useAppStore.getState();
  const showMarketplaceOverlay = useAppStore((s) => s.showMarketplaceOverlay);

  return (
    <AnimatePresence>
      {showMarketplaceOverlay && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.15 }}
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-2xl"
        >
          <div
            className="absolute inset-0"
            onClick={() => setShowMarketplaceOverlay(false)}
          />

          <motion.div
            initial={{ opacity: 0, scale: 0.95, y: 15 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.95, y: 15 }}
            transition={{ type: 'spring', stiffness: 350, damping: 25 }}
            className="w-[95vw] max-w-[1100px] h-[90vh] liquid-glass-panel flex flex-col overflow-hidden relative z-10"
          >
            {/* Header */}
            <div className="flex items-center justify-between px-4 py-3 border-b border-borderSubtle">
              <h2 className="text-lg font-semibold text-textPrimary flex items-center gap-2">
                <Store size={20} className="text-accent" />
                Marketplace
              </h2>
              <Tooltip content="Close" delay={200} side="left">
                <button
                  onClick={() => setShowMarketplaceOverlay(false)}
                  className="p-1.5 text-textSecondary hover:text-textPrimary hover:bg-glass rounded transition-all duration-200"
                >
                  <X size={18} />
                </button>
              </Tooltip>
            </div>

            {/* Content */}
            <div className="scrollbar-thin flex-1 overflow-y-auto px-5 py-5">
              <PluginsSettings />
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
