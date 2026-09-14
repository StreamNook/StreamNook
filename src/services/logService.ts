import { invoke } from '@tauri-apps/api/core';

// Log levels in order of severity
export type LogLevel = 'debug' | 'info' | 'warn' | 'error';

export interface LogEntry {
    timestamp: string;
    level: LogLevel;
    category: string;
    message: string;
    data?: unknown;
}

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

// Get all logs from Rust backend
export const getLogs = async (): Promise<LogEntry[]> => {
    try {
        return await invoke<LogEntry[]>('get_recent_logs', { limit: 500 });
    } catch (err) {
        originalConsole.error('[LogService] Failed to get logs:', err);
        return [];
    }
};

// Get logs filtered by level from Rust backend
export const getLogsByLevel = async (level: LogLevel): Promise<LogEntry[]> => {
    try {
        return await invoke<LogEntry[]>('get_logs_by_level', { level });
    } catch (err) {
        originalConsole.error('[LogService] Failed to get logs by level:', err);
        return [];
    }
};

// Get logs filtered by category (client-side filtering of fetched logs)
export const getLogsByCategory = async (category: string): Promise<LogEntry[]> => {
    try {
        const logs = await getLogs();
        return logs.filter(log => log.category.toLowerCase().includes(category.toLowerCase()));
    } catch (err) {
        originalConsole.error('[LogService] Failed to get logs by category:', err);
        return [];
    }
};

// Clear all logs in Rust backend
export const clearLogs = async (): Promise<void> => {
    try {
        await invoke('clear_logs');
    } catch (err) {
        originalConsole.error('[LogService] Failed to clear logs:', err);
    }
};

// Get unique categories from logs
export const getCategories = async (): Promise<string[]> => {
    try {
        const logs = await getLogs();
        return [...new Set(logs.map(log => log.category))];
    } catch (err) {
        originalConsole.error('[LogService] Failed to get categories:', err);
        return [];
    }
};

// Helper to format log data
const formatData = (data: unknown): string => {
    if (data === undefined) return '';
    try {
        return JSON.stringify(data, null, 2);
    } catch {
        return String(data);
    }
};

// Format logs for export
export const formatLogsForExport = (logsToExport: LogEntry[]): string => {
    return logsToExport.map(log => {
        const dataStr = log.data ? `\n  Data: ${formatData(log.data)}` : '';
        return `[${log.timestamp}] [${log.level.toUpperCase()}] [${log.category}] ${log.message}${dataStr}`;
    }).join('\n');
};

// Generate a full bug report
export const generateBugReport = async (): Promise<string> => {
    let appVersion = 'Unknown';
    let osInfo = 'Unknown';

    try {
        appVersion = await invoke('get_app_version') as string;
    } catch {
        // Ignore
    }

    try {
        osInfo = await invoke('get_system_info') as string;
    } catch {
        osInfo = navigator.userAgent;
    }

    const logs = await getLogs();
    const errorLogs = logs.filter(l => l.level === 'error' || l.level === 'warn');
    const recentLogs = logs.slice(-100); // Last 100 logs

    // The IRC lifecycle ring records every connect/JOIN/drop with its reason;
    // it is the primary evidence for chat-connection reports.
    let lifecycle: string[] = [];
    try {
        lifecycle = await invoke<string[]>('get_chat_lifecycle_log');
    } catch {
        // Ignore; the section renders as empty
    }

    const report = `
================================================================================
                         STREAMNOOK BUG REPORT
================================================================================

Generated: ${new Date().toISOString()}

--------------------------------------------------------------------------------
SYSTEM INFORMATION
--------------------------------------------------------------------------------
App Version: ${appVersion}
OS/Platform: ${osInfo}
User Agent: ${navigator.userAgent}

--------------------------------------------------------------------------------
ERROR/WARNING LOGS (${errorLogs.length} entries)
--------------------------------------------------------------------------------
${errorLogs.length > 0 ? formatLogsForExport(errorLogs) : 'No errors or warnings recorded.'}

--------------------------------------------------------------------------------
CHAT CONNECTION LIFECYCLE (${lifecycle.length} events)
--------------------------------------------------------------------------------
${lifecycle.length > 0 ? lifecycle.join('\n') : 'No lifecycle events recorded.'}

--------------------------------------------------------------------------------
RECENT ACTIVITY (Last 100 logs)
--------------------------------------------------------------------------------
${formatLogsForExport(recentLogs)}

================================================================================
                           END OF BUG REPORT
================================================================================
`;

    return report;
};

// Copy bug report to clipboard
export const copyBugReportToClipboard = async (): Promise<boolean> => {
    try {
        const report = await generateBugReport();
        await navigator.clipboard.writeText(report);
        return true;
    } catch (error) {
        originalConsole.error('Failed to copy bug report:', error);
        return false;
    }
};

// Save bug report to file (using browser download)
export const saveBugReportToFile = async (): Promise<boolean> => {
    try {
        const report = await generateBugReport();
        const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
        const filename = `streamnook-bug-report-${timestamp}.txt`;

        // Create a blob and download link
        const blob = new Blob([report], { type: 'text/plain' });
        const url = URL.createObjectURL(blob);

        const link = document.createElement('a');
        link.href = url;
        link.download = filename;
        document.body.appendChild(link);
        link.click();
        document.body.removeChild(link);

        // Clean up the URL
        URL.revokeObjectURL(url);

        return true;
    } catch (error) {
        originalConsole.error('Failed to save bug report:', error);
        return false;
    }
};

// Streamlink error detection (kept for compatibility)
const STREAMLINK_ERROR_PATTERNS = [
    /streamlink/i,
    /stream.*not.*found/i,
    /failed.*start.*stream/i,
    /no.*streams.*found/i,
    /executable.*not.*found/i,
];

export const isStreamlinkError = (errorMessage: string): boolean => {
    return STREAMLINK_ERROR_PATTERNS.some(pattern => pattern.test(errorMessage));
};

// Errors logged via console.error are captured by the Rust backend and written
// to a local crash log only. Nothing is sent off the user's machine.

export default {
    initLogCapture,
    trackActivity,
    getLogs,
    getLogsByLevel,
    getLogsByCategory,
    clearLogs,
    getCategories,
    formatLogsForExport,
    generateBugReport,
    copyBugReportToClipboard,
    saveBugReportToFile,
    isStreamlinkError,
};
