import { Component, ErrorInfo, ReactNode } from 'react';

import { Logger } from '../utils/logger';

export interface ErrorBoundaryFallbackArgs {
    error?: Error;
    componentName: string;
    /** Clears the error state and re-mounts the wrapped subtree. */
    reset: () => void;
}

interface Props {
    children: ReactNode;
    /** Static replacement UI. For a fallback that can recover, use
     *  `fallbackRender` so it receives the `reset` callback. */
    fallback?: ReactNode;
    /** Render-prop fallback. Receives a `reset` that clears the error and
     *  re-mounts the subtree, the caught error, and the boundary name. */
    fallbackRender?: (args: ErrorBoundaryFallbackArgs) => ReactNode;
    componentName?: string;
    reportToLogService?: boolean;
    /** When any entry changes while the boundary is in its error state, it
     *  resets automatically. Pass the channel/stream id so a failed panel
     *  recovers on its own the moment the user navigates away from the input
     *  that broke it. */
    resetKeys?: ReadonlyArray<unknown>;
    /** Called immediately before the error state clears (manual reset or a
     *  `resetKeys` change). Use it to drop stale local state that contributed
     *  to the failure. */
    onReset?: () => void;
}

interface State {
    hasError: boolean;
    error?: Error;
}

/**
 * Fault-isolation boundary. Each independent feature region (Home, video, chat,
 * mod logs, sidebar) gets its own so a render fault in one can never unwind into
 * a sibling: React only unwinds to the nearest boundary, so a contained failure
 * degrades that one panel instead of tearing down the whole tree.
 *
 * Unlike a bare boundary, this one can recover: it resets on a `resetKeys`
 * change and exposes a manual `reset` to its fallback, so a transient fault
 * self-heals without an app restart.
 */
class ErrorBoundary extends Component<Props, State> {
    constructor(props: Props) {
        super(props);
        this.state = { hasError: false };
    }

    static getDerivedStateFromError(error: Error): State {
        return { hasError: true, error };
    }

    componentDidCatch(error: Error, errorInfo: ErrorInfo) {
        const componentName = this.props.componentName || 'Component';

        if (this.props.reportToLogService) {
            // Log as error so it gets captured by logService and written to the local crash log
            Logger.error(`[${componentName}] Critical error caught by boundary:`, error);
            Logger.error(`[${componentName}] Component stack:`, errorInfo.componentStack);
        } else {
            // Log as warn (not error) so handled boundary errors don't fill the local crash log
            Logger.warn(`[${componentName}] Error caught and handled by boundary:`, error.message || error.toString());
            Logger.warn(`[${componentName}] Component stack:`, errorInfo.componentStack);
        }
    }

    componentDidUpdate(prevProps: Props) {
        if (!this.state.hasError) return;
        if (didResetKeysChange(prevProps.resetKeys, this.props.resetKeys)) {
            this.reset();
        }
    }

    reset = () => {
        this.props.onReset?.();
        this.setState({ hasError: false, error: undefined });
    };

    render() {
        if (this.state.hasError) {
            const componentName = this.props.componentName || 'Component';

            if (this.props.fallbackRender) {
                return this.props.fallbackRender({
                    error: this.state.error,
                    componentName,
                    reset: this.reset,
                });
            }

            if (this.props.fallback) {
                return this.props.fallback;
            }

            return (
                <div className="h-full w-full flex items-center justify-center p-4">
                    <div className="glass-panel w-full max-w-xs text-center px-5 py-4 rounded-xl">
                        <p className="text-textPrimary text-sm font-medium mb-1">
                            {componentName} stopped responding
                        </p>
                        <p className="text-textSecondary text-xs mb-3">
                            Reloading this panel usually clears it. The rest of the app keeps running.
                        </p>
                        <button
                            onClick={this.reset}
                            className="glass-button text-white text-xs font-medium px-3 py-1.5 rounded-lg border border-transparent hover:bg-white/10 transition-colors"
                        >
                            Reload {componentName}
                        </button>
                    </div>
                </div>
            );
        }

        return this.props.children;
    }
}

/** Shallow, order-sensitive compare of two reset-key arrays. */
function didResetKeysChange(
    prev: ReadonlyArray<unknown> | undefined,
    next: ReadonlyArray<unknown> | undefined,
): boolean {
    if (prev === next) return false;
    if (!prev || !next) return prev !== next;
    if (prev.length !== next.length) return true;
    for (let i = 0; i < prev.length; i++) {
        if (!Object.is(prev[i], next[i])) return true;
    }
    return false;
}

export default ErrorBoundary;
