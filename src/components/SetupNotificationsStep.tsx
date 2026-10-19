// The setup wizard's Notifications step on desktop, the phones' step with the
// desktop's own surfaces: one switch per alert type Rust offers in setup
// (`get_alert_types`, the ones marked `setup`), all off until chosen, then
// where they show (Dynamic Island, toasts or both). Turning a type on writes
// its setting, and on a Mac, the one desktop that asks, asks for the system's
// permission then; a refusal is said plainly, with the way to System Settings,
// hung under the choices so nothing moves. The phones draw the same step
// (src/mobile/setup/NotificationsStep.tsx, SetupWizard.swift).
import { Check, ExternalLink, Layers, MessageSquare, PanelTop } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import { Toggle } from './ui/Toggle';
import type { AlertType } from '../mobile/alertTypes';
import { setSettingsFields } from '../utils/settingsFields';
import { openSystemNotifySettings, useSystemNotifyPermission } from '../utils/systemNotifyPermission';
import { IS_MAC } from '../utils/platform';
import { Logger } from '../utils/logger';

// "Both" is just the two booleans on together; each choice maps onto the
// use_dynamic_island / use_toast pair the rest of the app already reads.
type NotifMode = 'island' | 'toast' | 'both';

const MODES: { id: NotifMode; icon: typeof PanelTop; label: string; desc: string }[] = [
    { id: 'island', icon: PanelTop, label: 'Dynamic Island', desc: 'Top of the window' },
    { id: 'toast', icon: MessageSquare, label: 'Toasts', desc: 'In a corner you pick' },
    { id: 'both', icon: Layers, label: 'Both', desc: 'Island and toast' },
];

interface Props {
    types: AlertType[];
    choices: Record<string, boolean>;
    onChoose: (id: string, on: boolean) => void;
}

const SetupNotificationsStep = ({ types, choices, onChoose }: Props) => {
    const { permission, ask } = useSystemNotifyPermission();
    const live = useAppStore((s) => s.settings.live_notifications);
    const islandOn = live?.use_dynamic_island ?? true;
    const toastOn = live?.use_toast ?? true;
    const mode: NotifMode = islandOn && toastOn ? 'both' : islandOn ? 'island' : toastOn ? 'toast' : 'both';

    const choose = async (type: AlertType, on: boolean) => {
        onChoose(type.id, on);
        // Choosing one also turns the master switch on, and the system's own
        // notifications with it, so a go-live or a mention reaches you while
        // StreamNook sits behind another app, as the phones' shade does.
        const changes: Record<string, unknown> = { [type.setting]: on };
        if (on) {
            changes['live_notifications.enabled'] = true;
            changes['live_notifications.use_native_notifications'] = true;
        }
        try {
            await setSettingsFields(changes);
        } catch (err) {
            Logger.warn('[SetupNotificationsStep] save failed:', err);
        }
        // A no-op off a Mac; on one it prompts only while never answered.
        if (on && permission !== 'granted') await ask();
    };

    const chooseMode = (next: NotifMode) => {
        setSettingsFields({
            'live_notifications.use_dynamic_island': next !== 'toast',
            'live_notifications.use_toast': next !== 'island',
        }).catch((err) => Logger.warn('[SetupNotificationsStep] save failed:', err));
    };

    const anyChosen = types.some((t) => choices[t.id]);
    const refused = anyChosen && permission === 'denied';

    return (
        <div className="relative w-full max-w-md text-left">
            <div className="rounded-xl border border-borderSubtle divide-y divide-borderSubtle">
                {types.map((type) => {
                    const on = choices[type.id] === true;
                    return (
                        <label key={type.id} className="flex cursor-pointer items-center gap-4 px-4 py-3 [@media(max-height:700px)]:py-2">
                            <span className="min-w-0 flex-1">
                                <span className="block text-sm font-semibold text-textPrimary">{type.title}</span>
                                <span className="block text-xs text-textSecondary">{type.detail}</span>
                            </span>
                            <Toggle enabled={on} onChange={() => void choose(type, !on)} ariaLabel={type.title} />
                        </label>
                    );
                })}
            </div>

            <p className="mt-6 mb-2 pl-[17px] text-xs font-medium text-textSecondary [@media(max-height:700px)]:mt-3 [@media(max-height:700px)]:mb-1.5">Show them in</p>
            <div className="grid grid-cols-3 gap-2">
                {MODES.map((opt) => {
                    const Icon = opt.icon;
                    const active = mode === opt.id;
                    return (
                        <button
                            key={opt.id}
                            type="button"
                            aria-pressed={active}
                            onClick={() => chooseMode(opt.id)}
                            className={`relative flex flex-col items-start gap-1.5 rounded-xl border px-4 py-3 text-left [@media(max-height:700px)]:gap-0.5 [@media(max-height:700px)]:py-2 transition-all duration-200 ${active ? 'border-accent bg-accent/10' : 'border-borderSubtle hover:border-borderLight'}`}
                        >
                            <Icon size={18} className={active ? 'text-accent' : 'text-textSecondary'} />
                            <span className="text-sm font-semibold text-textPrimary">{opt.label}</span>
                            <span className="text-xs leading-snug text-textSecondary">{opt.desc}</span>
                            {active && (
                                <Check size={14} strokeWidth={3} className="absolute right-4 top-3.5 text-accent" />
                            )}
                        </button>
                    );
                })}
            </div>

            {/* Only a Mac can refuse. The line hangs under the choices
                without taking layout room, so the centered page sits where it
                does on every desktop and nothing above it moves when it shows.
                The same banner the System notifications row shows in Settings.
                In a short window (the 600 px minimum) the whole step tightens
                by height alone and the banner keeps its room in the page, so it
                clears the footer; hidden or shown, every state draws the same. */}
            {IS_MAC && (
                <div
                    aria-hidden={!refused}
                    className={`absolute inset-x-0 top-full mt-5 flex items-center gap-3 rounded-lg border border-warning/30 bg-glass-raised/95 py-2.5 pl-4 pr-3 text-left [@media(max-height:700px)]:static [@media(max-height:700px)]:mt-3 [@media(max-height:700px)]:py-2 ${refused ? '' : 'invisible'}`}
                >
                    <p className="text-[12px] leading-relaxed text-textSecondary [@media(max-height:700px)]:leading-snug">
                        Notifications are turned off for StreamNook in macOS settings, so these alerts will not reach you
                        outside StreamNook until you allow them there.
                    </p>
                    <button
                        type="button"
                        tabIndex={refused ? 0 : -1}
                        onClick={openSystemNotifySettings}
                        className="glass-button flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-lg px-3 py-1.5 text-xs font-medium text-textPrimary"
                    >
                        Open macOS settings
                        <ExternalLink size={12} />
                    </button>
                </div>
            )}
        </div>
    );
};

export default SetupNotificationsStep;
