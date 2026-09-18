import { invoke } from '@tauri-apps/api/core';

// Log levels in order of severity
export type LogLevel = 'debug' | 'info' | 'warn' | 'error';

// Original console methods
const originalConsole = {
    log: console.log.bind(console),
    info: console.info.bind(console),
    warn: console.warn.bind(console),
    error: console.error.bind(console),
};

// Helper to extract category from message
const extractCategory = (message: string): { category: string; cleanMessage: string } => {
    const match = message.match(/^\[([^\]]+)\]\s*(.*)$/);
    if (match) {
        return { category: match[1], cleanMessage: match[2] };
    }
    return { category: 'App', cleanMessage: message };
};

// Forward queue: a library warn/error storm (a player during buffer
// degradation prints dozens of lines in one tick) used to cost one IPC round
// trip per line, exactly when the main thread was already stressed. Lines
// coalesce for up to 500ms into a single batched invoke; the cap flushes a
// storm early and pagehide flushes teardown. Worst case a hard webview crash
// loses the last <=500ms of lines; the Rust side keeps its own log.
type QueuedForwardLine = {
    level: LogLevel;
    category: string;
    message: string;
    data: string | null;
};
const forwardQueue: QueuedForwardLine[] = [];
let forwardTimer: ReturnType<typeof setTimeout> | null = null;
const FORWARD_FLUSH_MS = 500;
const FORWARD_QUEUE_CAP = 200;

const flushForwardQueue = (): void => {
    if (forwardTimer !== null) {
        clearTimeout(forwardTimer);
        forwardTimer = null;
    }
    if (forwardQueue.length === 0) return;
    const entries = forwardQueue.splice(0);
    invoke('log_messages_batch', { entries }).catch((err) => {
        // Silent fail - don't log errors about logging
        originalConsole.warn('[LogService] Failed to forward log batch to Rust:', err);
    });
};

try {
    window.addEventListener('pagehide', flushForwardQueue);
} catch {
    /* non-DOM context */
}

// Forward log to Rust backend. Exported so utils/logger.ts can forward its
// lines directly: Logger binds the native console at module load (before
// initLogCapture patches it), so its output never reaches the patched console
// and this is its only path into the backend log file.
export const forwardToRust = async (level: LogLevel, args: unknown[]): Promise<void> => {
    try {
        const firstArg = String(args[0] || '');
        const { category, cleanMessage } = extractCategory(firstArg);
        const data = args.length > 1 ? args.slice(1) : undefined;

        forwardQueue.push({
            level,
            category,
            message: cleanMessage || firstArg,
            data: data ? JSON.stringify(data) : null,
        });
        if (forwardQueue.length >= FORWARD_QUEUE_CAP) {
            flushForwardQueue();
        } else if (forwardTimer === null) {
            forwardTimer = setTimeout(flushForwardQueue, FORWARD_FLUSH_MS);
        }
    } catch (err) {
        // Silent fail - don't log errors about logging
        originalConsole.warn('[LogService] Failed to queue log for Rust:', err);
    }
};

// Initialize the log capture by wrapping console methods
// Forwards warnings/errors to the Rust backend for local storage and crash logging
export const initLogCapture = (): void => {
    console.log = (...args: unknown[]) => {
        // Don't forward regular logs to backend - only WARN and ERROR are meaningful for debugging
        originalConsole.log(...args);
    };

    console.info = (...args: unknown[]) => {
        // Don't store info logs, just pass through
        originalConsole.info(...args);
    };

    console.warn = (...args: unknown[]) => {
        forwardToRust('warn', args);
        originalConsole.warn(...args);
    };

    console.error = (...args: unknown[]) => {
        forwardToRust('error', args);
        originalConsole.error(...args);
    };
};

// Track user activity for error context
export const trackActivity = async (action: string): Promise<void> => {
    try {
        await invoke('track_activity', { action });
    } catch (err) {
        originalConsole.warn('[LogService] Failed to track activity:', err);
    }
};

