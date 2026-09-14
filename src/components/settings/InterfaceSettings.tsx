import { useState, useEffect } from 'react';
import { Eye, EyeOff, Columns, X, Sparkles, Gauge, Zap } from 'lucide-react';
import CompactViewSettings from './CompactViewSettings';
import { SettingsSection, SettingsRow, SegmentedSelect } from './_primitives';
import { GlassMultiSelect } from '../ui/GlassMultiSelect';
import { DISCOVERY_LANGUAGES } from '../../utils/discoveryLanguages';
import { useAppStore } from '../../stores/AppStore';
import type { MotionMode, CloseToTrayMode } from '../../types';

export type SidebarMode = 'expanded' | 'compact' | 'hidden' | 'disabled';

export const getSidebarSettings = () => {
    const mode = localStorage.getItem('sidebar-mode') as SidebarMode | null;
    const expandOnHover = localStorage.getItem('sidebar-expand-on-hover');
    const showRecommended = localStorage.getItem('sidebar-show-recommended');

    return {
        mode: mode || 'compact',
        expandOnHover: expandOnHover ? JSON.parse(expandOnHover) : true,
        showRecommended: showRecommended ? JSON.parse(showRecommended) : true
    };
};

export const saveSidebarSettings = (mode: SidebarMode, expandOnHover: boolean, showRecommended: boolean) => {
    localStorage.setItem('sidebar-mode', mode);
    localStorage.setItem('sidebar-expand-on-hover', JSON.stringify(expandOnHover));
    localStorage.setItem('sidebar-show-recommended', JSON.stringify(showRecommended));

    window.dispatchEvent(new CustomEvent('sidebar-settings-changed', {
        detail: { mode, expandOnHover, showRecommended }
    }));
};

const Toggle = ({ enabled, onChange }: { enabled: boolean; onChange: () => void }) => (
    <button
        onClick={onChange}
        className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors flex-shrink-0 ${enabled ? 'bg-accent' : 'bg-gray-600'
            }`}
    >
        <span
            className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${enabled ? 'translate-x-6' : 'translate-x-1'
                }`}
        />
    </button>
);

const SIDEBAR_MODE_OPTIONS: { value: SidebarMode; label: string; hint: string; Icon: typeof Columns }[] = [
    { value: 'expanded', label: 'Expanded', hint: 'Always show full sidebar', Icon: Columns },
    { value: 'compact', label: 'Compact', hint: 'Show avatars only', Icon: Eye },
    { value: 'hidden', label: 'Hidden', hint: 'Show on hover only', Icon: EyeOff },
    { value: 'disabled', label: 'Disabled', hint: 'Completely hidden', Icon: X },
];

const MOTION_MODE_OPTIONS: { value: MotionMode; label: string; hint: string; Icon: typeof Columns }[] = [
    { value: 'full', label: 'Full', hint: 'All animations', Icon: Sparkles },
    { value: 'reduced', label: 'Reduced', hint: 'Fades only', Icon: Gauge },
    { value: 'off', label: 'Off', hint: 'Instant, snappy', Icon: Zap },
];

const InterfaceSettings = () => {
    const { settings, updateSettings } = useAppStore();
    const [sidebarMode, setSidebarMode] = useState<SidebarMode>('compact');
    const [expandOnHover, setExpandOnHover] = useState(true);
    const [showRecommended, setShowRecommended] = useState(true);

    // Compact (centered window) is the default; turning it off grows Settings
    // to a full-page layout that fills the entire app.
    const compactSettingsWindow = settings.compact_settings_window !== false;
    const handleCompactSettingsWindowChange = (enabled: boolean) => {
        void updateSettings({ ...settings, compact_settings_window: enabled });
    };

    // Full (default) animates everything; Reduced keeps fades but drops
    // movement; Off makes the UI instant and snappy (best on low-end PCs).
    const closeToTray: CloseToTrayMode = settings.close_to_tray ?? 'with-popouts';
    const closeToTrayDescription = (() => {
        switch (closeToTray) {
            case 'always':
                return 'Closing the window always leaves StreamNook running in the system tray. Quit it from the tray icon.';
            case 'never':
                return 'Closing the window always quits StreamNook, even while MultiChat popouts are open.';
            default:
                return 'Closing the window quits StreamNook, unless MultiChat popouts are still open, in which case it minimizes to the system tray so they keep working.';
        }
    })();

    const keepOnTopInCompact = settings.keep_on_top_in_compact === true;

    const motionMode: MotionMode = settings.motion_mode ?? 'full';
    const handleMotionModeChange = (mode: MotionMode) => {
        void updateSettings({ ...settings, motion_mode: mode });
    };
    const motionDescription = (() => {
        switch (motionMode) {
            case 'full':
                return 'Every animation and transition plays normally.';
            case 'reduced':
                return 'Keeps quick fades but removes sliding, scaling, and bouncing, which is easier on the eyes and lighter on slower PCs.';
            case 'off':
                return 'Turns animations off for an instant, snappy feel, and loading spinners still spin.';
        }
    })();

    useEffect(() => {
        const settings = getSidebarSettings();
        queueMicrotask(() => {
            setSidebarMode(settings.mode);
            setExpandOnHover(settings.expandOnHover);
            setShowRecommended(settings.showRecommended);
        });
    }, []);

    const handleModeChange = (mode: SidebarMode) => {
        setSidebarMode(mode);
        saveSidebarSettings(mode, expandOnHover, showRecommended);
    };

    const handleExpandOnHoverChange = (enabled: boolean) => {
        setExpandOnHover(enabled);
        saveSidebarSettings(sidebarMode, enabled, showRecommended);
    };

    const handleShowRecommendedChange = (enabled: boolean) => {
        setShowRecommended(enabled);
        saveSidebarSettings(sidebarMode, expandOnHover, enabled);
    };

    const applyDiscoveryLanguages = async (languages: string[]) => {
        // Read the store fresh at write time so rapid toggles never clobber a
        // setting changed elsewhere in the meantime. The await is load-bearing:
        // updateSettings commits to the store only after the save completes,
        // and the reload below reads the filter back out of the store.
        const current = useAppStore.getState().settings;
        await updateSettings({ ...current, discovery_languages: languages });
        void useAppStore.getState().loadRecommendedStreams();
    };

    const applyDiscoveryPersonalized = async (enabled: boolean) => {
        const current = useAppStore.getState().settings;
        await updateSettings({ ...current, discovery_personalized: enabled });
        void useAppStore.getState().loadRecommendedStreams();
    };

    const modeDescription = (() => {
        switch (sidebarMode) {
            case 'expanded':
                return 'The sidebar stays fully open, with streamer names, categories, and viewer counts always in view.';
            case 'compact':
                return `Only profile pictures show, ${expandOnHover ? 'and hovering reveals the full details' : 'and the arrow expands it when you need more'}.`;
            case 'hidden':
                return 'The sidebar stays out of the way until you move your cursor to the left edge of the window, and it stays open while your cursor is over it.';
            case 'disabled':
                return 'The sidebar never appears, for the cleanest possible layout without the streams list.';
        }
    })();

    return (
        <div className="space-y-8">
            <SettingsSection
                id="settings-section-sidebar"
                label="Sidebar"
                description="How much of the streams list stays on screen while you watch."
            >
                <SettingsRow
                    title="How the sidebar appears"
                    description={modeDescription}
                >
                    <div className="grid grid-cols-4 gap-2">
                        {SIDEBAR_MODE_OPTIONS.map(({ value, label, hint, Icon }) => {
                            const isActive = sidebarMode === value;
                            return (
                                <button
                                    key={value}
                                    onClick={() => handleModeChange(value)}
                                    style={{ borderRadius: 8 }}
                                    className={`flex flex-col items-center gap-2 p-3 text-sm font-medium transition-all ${isActive
                                        ? 'glass-input text-textPrimary'
                                        : 'glass-button text-textSecondary hover:text-textPrimary'
                                        }`}
                                >
                                    <Icon size={24} />
                                    <span className="text-xs font-medium">{label}</span>
                                    <span className="text-[10px] text-textMuted text-center">{hint}</span>
                                </button>
                            );
                        })}
                    </div>
                </SettingsRow>

                {sidebarMode === 'compact' && (
                    <SettingsRow
                        title="Expand when you hover"
                        description="Move your cursor over the compact sidebar to open it fully, and it folds back when you leave."
                        control={
                            <Toggle
                                enabled={expandOnHover}
                                onChange={() => handleExpandOnHoverChange(!expandOnHover)}
                            />
                        }
                    />
                )}

                <SettingsRow
                    title="Show recommended streams"
                    description="Show the Recommended section in the sidebar. Turn this off to keep only your followed channels and favorites."
                    control={
                        <Toggle
                            enabled={showRecommended}
                            onChange={() => handleShowRecommendedChange(!showRecommended)}
                        />
                    }
                />
            </SettingsSection>

            <SettingsSection
                id="settings-section-discover"
                label="Discover Feed"
                description="What the Discover tab and the sidebar's Recommended section show."
            >
                <SettingsRow
                    title="Personalized recommendations"
                    description="Use your Twitch account to tailor Discover to what you watch. When off, recommendations are anonymous and based only on your region."
                    control={
                        <Toggle
                            enabled={settings.discovery_personalized ?? false}
                            onChange={() => void applyDiscoveryPersonalized(!(settings.discovery_personalized ?? false))}
                        />
                    }
                />

                <SettingsRow
                    title="Languages"
                    description="Only show recommended streams in these languages. Empty means any language."
                    onReset={(settings.discovery_languages?.length ?? 0) > 0 ? () => void applyDiscoveryLanguages([]) : undefined}
                    control={
                        <GlassMultiSelect
                            values={settings.discovery_languages ?? []}
                            onChange={(v) => void applyDiscoveryLanguages(v)}
                            options={DISCOVERY_LANGUAGES}
                            emptyLabel="Any language"
                        />
                    }
                />
            </SettingsSection>

            <SettingsSection
                id="settings-section-motion"
                label="Motion"
                description="How much the interface moves, from full animation to instant."
            >
                <SettingsRow
                    title="How much the interface animates"
                    description={motionDescription}
                    help="Off is the best choice on a low-end PC, since the frosted-glass blur is expensive to animate."
                >
                    <div className="grid grid-cols-3 gap-2">
                        {MOTION_MODE_OPTIONS.map(({ value, label, hint, Icon }) => {
                            const isActive = motionMode === value;
                            return (
                                <button
                                    key={value}
                                    onClick={() => handleMotionModeChange(value)}
                                    style={{ borderRadius: 8 }}
                                    className={`flex flex-col items-center gap-2 p-3 text-sm font-medium transition-all ${isActive
                                        ? 'glass-input text-textPrimary'
                                        : 'glass-button text-textSecondary hover:text-textPrimary'
                                        }`}
                                >
                                    <Icon size={24} />
                                    <span className="text-xs font-medium">{label}</span>
                                    <span className="text-[10px] text-textMuted text-center">{hint}</span>
                                </button>
                            );
                        })}
                    </div>
                </SettingsRow>
            </SettingsSection>

            <SettingsSection
                id="settings-section-window-close"
                label="Closing the Window"
                description="Whether the close button quits StreamNook or tucks it into the system tray."
            >
                <SettingsRow
                    title="What the close button does"
                    description={closeToTrayDescription}
                >
                    <SegmentedSelect<CloseToTrayMode>
                        value={closeToTray}
                        onChange={(mode) => void updateSettings({ ...settings, close_to_tray: mode })}
                        options={[
                            { value: 'with-popouts', label: 'Only with popouts' },
                            { value: 'always', label: 'Always minimize' },
                            { value: 'never', label: 'Always quit' },
                        ]}
                    />
                </SettingsRow>
            </SettingsSection>

            <SettingsSection
                id="settings-section-settings-window"
                label="Settings Window"
                description="How much room this settings screen takes up."
            >
                <SettingsRow
                    title="Keep settings in a centered window"
                    description="Settings open in a centered window; turn this off to open them as a full page that fills the app."
                    help="The full page gives long tabs more room, so you see more options at once with less scrolling."
                    control={
                        <Toggle
                            enabled={compactSettingsWindow}
                            onChange={() => handleCompactSettingsWindowChange(!compactSettingsWindow)}
                        />
                    }
                />
            </SettingsSection>

            <SettingsSection
                id="settings-section-window-on-top"
                label="Keep on Top"
                description="Keep the small Compact View player visible over whatever else you are doing."
            >
                <SettingsRow
                    title="Keep on top in Compact View"
                    description="While Compact View is active, the small player floats above other apps so clicking your browser does not bury it."
                    help="The window drops back to normal as soon as you leave Compact View."
                    control={
                        <Toggle
                            enabled={keepOnTopInCompact}
                            onChange={() => void updateSettings({ ...settings, keep_on_top_in_compact: !keepOnTopInCompact })}
                        />
                    }
                />
            </SettingsSection>

            <div id="settings-section-compact">
                <CompactViewSettings />
            </div>
        </div>
    );
};

export default InterfaceSettings;
