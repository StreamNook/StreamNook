import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { motion, AnimatePresence } from 'framer-motion';
import { KeyRound } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';

interface ConsentRequest {
  request_id: string;
  plugin_id: string;
  plugin_name: string;
  kind: string;
}

/**
 * Mounted once at the app root. Bridges plugin-host runtime events into the
 * UI: the blocking credential consent prompt, plugin notifications, and the
 * disabled-after-failures notice. Renders nothing unless a consent prompt is
 * pending.
 */
const PluginRuntimeBridge = () => {
  const addToast = useAppStore((s) => s.addToast);
  const [request, setRequest] = useState<ConsentRequest | null>(null);

  useEffect(() => {
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    const setup = async () => {
      const un1 = await listen<ConsentRequest>('plugin://consent-request', (event) => {
        setRequest(event.payload);
      });
      const un2 = await listen<{ plugin_name: string; level: string; message: string }>(
        'plugin://notify',
        (event) => {
          const { plugin_name, level, message } = event.payload;
          const toastLevel = level === 'warning' ? 'info' : level === 'error' ? 'error' : 'info';
          addToast(`${plugin_name}: ${message}`, toastLevel as 'info' | 'error', undefined, {
            // So the notification centre marks it as a plugin rather than as
            // one more status line.
            source: 'plugin',
          });
        }
      );
      const un3 = await listen<{ name: string; reason: string }>(
        'plugin://disabled-after-failures',
        (event) => {
          addToast(
            `Plugin "${event.payload.name}" was disabled after repeated failures`,
            'error'
          );
        }
      );
      if (disposed) {
        un1();
        un2();
        un3();
      } else {
        unlisteners.push(un1, un2, un3);
      }
    };
    setup();
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [addToast]);

  const respond = async (decision: 'allow' | 'always' | 'deny') => {
    if (!request) return;
    try {
      await invoke('plugins_respond_consent', {
        requestId: request.request_id,
        decision,
      });
    } catch (err) {
      Logger.error('[Plugins] consent response failed:', err);
    }
    setRequest(null);
  };

  return (
    <AnimatePresence>
      {request && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.15 }}
          className="fixed inset-0 z-[300] flex items-center justify-center bg-black/60 backdrop-blur-sm"
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 12 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 12 }}
            transition={{ type: 'spring', stiffness: 380, damping: 30 }}
            className="glass-panel relative w-[440px] max-w-[90vw] p-6"
          >
            <div className="flex items-start gap-3.5">
              <div
                className="flex h-11 w-11 flex-shrink-0 items-center justify-center rounded-xl"
                style={{
                  background: 'rgba(225, 130, 130, 0.16)',
                  boxShadow:
                    'inset 1px 1px 0 0 rgba(255,255,255,0.10), inset -1px -1px 0 0 rgba(0,0,0,0.18)',
                }}
              >
                <KeyRound size={20} className="text-red-300" />
              </div>
              <div className="min-w-0 flex-1">
                <h2 className="text-[15px] font-bold leading-snug text-textPrimary">
                  {request.plugin_name} is asking to use your Twitch login.
                </h2>
                <p className="mt-2 text-[13px] leading-relaxed text-textSecondary">
                  If you allow this, the plugin receives a token that lets it act as
                  your Twitch account: watching, claiming, and anything else that
                  login can do. StreamNook records every time a credential is handed
                  to a plugin.
                </p>
              </div>
            </div>
            <div className="mt-5 flex items-center justify-end gap-2">
              <button
                type="button"
                onClick={() => respond('deny')}
                className="glass-button-secondary px-4 py-2 text-[13px] text-textSecondary hover:text-textPrimary"
              >
                Deny
              </button>
              <button
                type="button"
                onClick={() => respond('always')}
                className="glass-button-secondary px-4 py-2 text-[13px] text-textSecondary hover:text-textPrimary"
              >
                Don't ask again
              </button>
              <button
                type="button"
                onClick={() => respond('allow')}
                className="glass-button-secondary !bg-accent/15 px-4 py-2 text-[13px] font-medium text-textPrimary hover:!bg-accent/25"
              >
                Allow
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};

export default PluginRuntimeBridge;
