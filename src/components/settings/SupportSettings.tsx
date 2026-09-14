import { useState, useEffect, type CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { DiscordGlyph } from '../ui/DiscordGlyph';
import streamnookLogo from '../../assets/streamnook-logo.png';
import { SettingsSection, SettingsRow } from './_primitives';
import { useAppStore } from '../../stores/AppStore';

import { Logger } from '../../utils/logger';

const Toggle = ({ enabled, onChange }: { enabled: boolean; onChange: () => void }) => (
    <button
        onClick={onChange}
        className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors flex-shrink-0 ${enabled ? 'bg-accent' : 'bg-gray-600'}`}
    >
        <span
            className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${enabled ? 'translate-x-6' : 'translate-x-1'}`}
        />
    </button>
);

const COMMUNITY_DISCORD_INVITE_CODE = '2xvuF9TES7';
const COMMUNITY_DISCORD_INVITE = `https://discord.gg/${COMMUNITY_DISCORD_INVITE_CODE}`;

// The server's live Discord banner is stale and can't be refreshed without more
// server boosts, so the card header is overridden with this CDN-hosted banner
// (R2 via cdn.streamnook.app — kept OFF the app bundle so it's swappable without
// shipping a release). Set to null to use the live Discord banner instead; and if
// the CDN URL ever fails to load, it falls back to the live banner at runtime too.
const CUSTOM_BANNER: string | null = 'https://cdn.streamnook.app/community-banner.gif';

interface DiscordInviteData {
    guild?: {
        id: string;
        name: string;
        icon: string | null;
        banner?: string | null;
        description?: string | null;
    };
    approximate_member_count?: number;
    approximate_presence_count?: number;
}

// Inline -webkit clamp so we don't depend on the Tailwind line-clamp utility
// being enabled in this project's config.
const clamp = (lines: number): CSSProperties => ({
    display: '-webkit-box',
    WebkitLineClamp: lines,
    WebkitBoxOrient: 'vertical',
    overflow: 'hidden',
});

const SupportSettings = () => {
    const [serverData, setServerData] = useState<DiscordInviteData | null>(null);
    // If the CDN banner fails to load, fall back to the live Discord banner.
    const [customBannerFailed, setCustomBannerFailed] = useState(false);

    useEffect(() => {
        const fetchServerData = async () => {
            try {
                const response = await fetch(
                    `https://discord.com/api/v10/invites/${COMMUNITY_DISCORD_INVITE_CODE}?with_counts=true`
                );
                if (response.ok) {
                    setServerData(await response.json());
                }
            } catch (error) {
                Logger.error('Failed to fetch Discord server preview:', error);
            }
        };

        fetchServerData();
        const interval = setInterval(fetchServerData, 60000);
        return () => clearInterval(interval);
    }, []);

    const getServerIconUrl = (guildId: string, iconHash: string) => {
        const extension = iconHash.startsWith('a_') ? 'gif' : 'png';
        return `https://cdn.discordapp.com/icons/${guildId}/${iconHash}.${extension}?size=128`;
    };

    const getServerBannerUrl = (guildId: string, bannerHash: string) => {
        const extension = bannerHash.startsWith('a_') ? 'gif' : 'png';
        return `https://cdn.discordapp.com/banners/${guildId}/${bannerHash}.${extension}?size=512`;
    };

    const handleJoinCommunity = async () => {
        try {
            const { open } = await import('@tauri-apps/plugin-shell');
            await open(COMMUNITY_DISCORD_INVITE);
        } catch (err) {
            Logger.error('Failed to open Discord invite:', err);
            window.open(COMMUNITY_DISCORD_INVITE, '_blank');
        }
    };

    const guild = serverData?.guild;
    const name = guild?.name ?? 'StreamNook';
    const iconUrl = guild?.icon ? getServerIconUrl(guild.id, guild.icon) : null;
    const discordBanner = guild?.banner ? getServerBannerUrl(guild.id, guild.banner) : null;
    const bannerUrl = CUSTOM_BANNER && !customBannerFailed ? CUSTOM_BANNER : discordBanner;
    const description = guild?.description ?? null;
    const presence = serverData?.approximate_presence_count;
    const members = serverData?.approximate_member_count;
    const hasCounts = typeof presence === 'number' && typeof members === 'number';

    // Matches the card's own surface so the overlapping icon reads as cut into
    // the card (theme-aware, not a fixed dark value).
    const cutoutBorder = { borderColor: 'var(--color-background-tertiary)' } as CSSProperties;

    return (
        <div className="flex min-h-full flex-col items-center py-6">
            {/* my-auto centers the card group vertically when there's room, and degrades
                to top-aligned + scrollable when the card is taller than the pane — unlike
                justify-center, which clips the top of an overflowing card out of reach. */}
            <div className="my-auto flex w-full max-w-[400px] flex-col items-center">
                {/* Intro — anchors the tab so the page reads as a deliberate, centered
                    invite screen rather than one stray bar across a wide empty page. */}
                <div className="mb-5 flex max-w-[340px] flex-col items-center text-center">
                <div className="mb-3 flex h-14 w-14 items-center justify-center rounded-2xl bg-accent/10 p-2.5">
                    <img src={streamnookLogo} alt="StreamNook" className="h-full w-full object-contain" />
                </div>
                <h2 className="text-[17px] font-semibold text-textPrimary">Join the community</h2>
                <p className="mt-1.5 text-[13px] leading-relaxed text-textSecondary">
                    Get help, request features, hear about updates, and hang out with other StreamNook users.
                </p>
            </div>

                {/* Discord invite preview card */}
                <div className="glass-panel w-full overflow-hidden text-left">
                {/* Header — the server banner when one exists, otherwise a soft
                    on-brand tint. A gently letterboxed 2:1 strip: shows nearly the
                    whole banner while keeping the card compact. A bottom scrim keeps
                    the icon/name legible over any banner art (legibility, not glow). */}
                <div className="relative aspect-[2/1]">
                    {bannerUrl ? (
                        <img
                            src={bannerUrl}
                            alt=""
                            className="absolute inset-0 h-full w-full object-cover"
                            onError={() => setCustomBannerFailed(true)}
                        />
                    ) : (
                        <div className="absolute inset-0" style={{ background: 'rgba(215, 165, 140, 0.07)' }} />
                    )}
                    <div className="absolute inset-0 bg-gradient-to-t from-black/55 to-transparent" />
                </div>

                <div className="px-4 pb-4">
                    {/* Server icon, overlapping the header */}
                    <div className="-mt-9 mb-3 flex">
                        <div className="flex-shrink-0">
                            {/* Frosted glass lives on the container itself, not a child layer:
                                an element's own backdrop-filter is clipped by its own radius,
                                whereas a child backdrop-filter layer isn't clipped by the parent's
                                rounded corners and its square edges poke through. Transparent areas
                                of the icon then rest on a plain frosted blur (no color bleed); an
                                opaque square icon simply covers it. */}
                            <div
                                className={`relative h-[68px] w-[68px] overflow-hidden rounded-2xl border ${
                                    iconUrl ? 'bg-white/[0.04] backdrop-blur-md' : ''
                                }`}
                                style={cutoutBorder}
                            >
                                {iconUrl ? (
                                    <img
                                        src={iconUrl}
                                        alt={name}
                                        className="absolute inset-0 h-full w-full object-contain"
                                    />
                                ) : (
                                    <div className="flex h-full w-full items-center justify-center bg-accent/10 p-3">
                                        <img
                                            src={streamnookLogo}
                                            alt="StreamNook"
                                            className="h-full w-full object-contain"
                                        />
                                    </div>
                                )}
                            </div>
                        </div>
                    </div>

                    {/* Name + invite handle */}
                    <h3 className="truncate text-[15px] font-semibold text-textPrimary">{name}</h3>
                    <p className="mt-0.5 text-[11px] font-medium uppercase tracking-[0.08em] text-textMuted">
                        discord.gg/{COMMUNITY_DISCORD_INVITE_CODE}
                    </p>

                    {description && (
                        <p className="mt-2.5 text-[12.5px] leading-relaxed text-textSecondary" style={clamp(2)}>
                            {description}
                        </p>
                    )}

                    {/* Online / member counts */}
                    {hasCounts && (
                        <div className="mt-3 flex items-center gap-4">
                            <span className="flex items-center gap-1.5 text-[12px] text-textSecondary">
                                {/* The live pulse rides the online count now — a Discord server
                                    is always "online", so a presence dot on its icon was noise. */}
                                <span className="relative flex h-2 w-2">
                                    <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-green-500 opacity-60" />
                                    <span className="relative inline-flex h-2 w-2 rounded-full bg-green-500" />
                                </span>
                                <span className="font-semibold text-textPrimary">
                                    {presence!.toLocaleString()}
                                </span>{' '}
                                Online
                            </span>
                            <span className="flex items-center gap-1.5 text-[12px] text-textSecondary">
                                <span className="h-2 w-2 rounded-full bg-textMuted/50" />
                                <span className="font-semibold text-textPrimary">
                                    {members!.toLocaleString()}
                                </span>{' '}
                                Members
                            </span>
                        </div>
                    )}

                    <button
                        onClick={handleJoinCommunity}
                        className="discord-join-button mt-4 flex w-full items-center justify-center gap-2 rounded-lg px-4 py-2.5 text-[13px] font-semibold text-white"
                    >
                        <DiscordGlyph size={17} />
                        Join the Discord
                    </button>
                </div>
                </div>
            </div>

            <DiagnosticLoggingSection />
        </div>
    );
};

/**
 * The diagnostic logging toggle.
 *
 * `error_reporting_enabled` already drove `set_diagnostics_enabled` from
 * AppStore, but it had no control anywhere in the app, so the only way to raise
 * the log level was to edit settings.json by hand. That is precisely the moment
 * someone needs it: they are trying to produce a log for a bug report and the
 * file is empty apart from the watchdog line.
 *
 * Lives on Support because that is where a person goes when something is wrong,
 * and it is the page a maintainer will point them at.
 */
const DiagnosticLoggingSection = () => {
    const { settings, updateSettings } = useAppStore();
    // Absent means ON, matching how AppStore reads it (`!== false`), so an
    // existing install is not silently switched off by adding this control.
    const enabled = settings.error_reporting_enabled !== false;

    return (
        <SettingsSection
            id="settings-section-diagnostics"
            label="Diagnostics"
            description="How much StreamNook writes to its log file, and where to find that file when someone asks you for it."
        >
            <SettingsRow
                title="Keep a detailed log for bug reports"
                description="Records connection, playback, and chat activity to streamnook.log on this PC so a problem can be traced after the fact."
                help="Leave it on if you might report a bug; with it off the log holds almost nothing worth sending. The file stays on your PC until you choose to share it."
                control={
                    <Toggle
                        enabled={enabled}
                        onChange={() => updateSettings({ ...settings, error_reporting_enabled: !enabled })}
                    />
                }
            />
            <SettingsRow
                title="Find the log file"
                description="Opens the folder that holds streamnook.log so you can attach it to a bug report."
                control={
                    <button
                        onClick={() => {
                            void invoke('open_logs_folder').catch((e) =>
                                Logger.warn('[Support] open logs folder failed:', e),
                            );
                        }}
                        className="px-3 py-1.5 rounded-md text-sm text-textPrimary bg-white/[0.06] hover:bg-white/[0.1] transition-colors"
                    >
                        Open logs folder
                    </button>
                }
            />
        </SettingsSection>
    );
};

export default SupportSettings;
