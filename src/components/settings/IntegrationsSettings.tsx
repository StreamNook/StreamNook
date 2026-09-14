import { Plug } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { usePluginUiRegistry, selectSlot } from '../../plugins-ui/registry';
import { DiscordGlyph } from '../ui/DiscordGlyph';
import { Toggle } from '../ui/Toggle';
import streamnookLogo from '../../assets/streamnook-logo-128.webp';

const IntegrationsSettings = () => {
  const { settings, updateSettings } = useAppStore();
  // Plugins contribute their own integration panels here, the same way a drops
  // plugin contributes into the Drops center's settings slot. The tab renders
  // whatever is contributed and names none of it; with no such plugin installed
  // the slot is empty and only the built-in integrations show.
  const pluginPanels = usePluginUiRegistry(selectSlot('integrations.settings'));

  return (
    <div className="flex min-h-full flex-col items-center py-6">
      {/* my-auto centers the group vertically when there's room and degrades to
          top-aligned + scrollable when the content outgrows the pane — unlike
          justify-center, which clips the top out of reach. */}
      <div className="my-auto flex w-full max-w-[640px] flex-col items-center">
        {/* Intro — anchors the tab so a single integration reads as a deliberate,
            centered screen rather than one stray row across a wide empty page. */}
        <div className="mb-5 flex max-w-[340px] flex-col items-center text-center">
        <div className="mb-3 flex h-14 w-14 items-center justify-center rounded-2xl bg-accent/10 text-accent">
          <Plug className="h-6 w-6" />
        </div>
        <h2 className="text-[17px] font-semibold text-textPrimary">Integrations</h2>
        <p className="mt-1.5 text-[13px] leading-relaxed text-textSecondary">
          Connect StreamNook with the apps you already use.
        </p>
      </div>

        {/* Kick and YouTube used to be listed here, which made them read as
            integrations alongside Discord. They are accounts — they live in
            Settings → Profile → Accounts with the Twitch ones. Integrations is
            Discord RPC and plugins. */}

        {/* Integration cards (room to stack as more land here) */}
        <div className="w-full space-y-3">
        <div className="glass-panel rounded-lg p-4">
          <div className="flex items-center gap-3.5">
            {/* Just the two marks with a + between them, signalling StreamNook and
                Discord working together — no tiles, aligned on one line. */}
            <div className="flex flex-shrink-0 items-center gap-1.5">
              <img
                src={streamnookLogo}
                alt="StreamNook"
                className="h-7 w-7 object-contain"
              />
              <span className="text-[13px] font-medium text-textMuted">+</span>
              <DiscordGlyph size={26} className="text-[#5865F2]" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="text-[14px] font-semibold text-textPrimary">Discord Rich Presence</div>
              <p className="mt-0.5 text-[12px] leading-relaxed text-textSecondary">
                Show what you're watching on your Discord profile.
              </p>
            </div>
            <Toggle
              enabled={settings.discord_rpc_enabled}
              onChange={() =>
                updateSettings({ ...settings, discord_rpc_enabled: !settings.discord_rpc_enabled })
              }
            />
          </div>
        </div>
        {pluginPanels.map((c) => {
          return (
            <div
              key={`${c.pluginId}:${c.id}`}
              className="space-y-2 border-t border-white/[0.06] pt-3"
            >
              <div className="flex items-center gap-2 px-1">
                <c.Icon size={14} className="text-accent" />
                <span className="text-[11px] uppercase tracking-[0.12em] text-textMuted">
                  {c.label}
                </span>
              </div>
              <c.Component />
            </div>
          );
        })}
        </div>
      </div>
    </div>
  );
};

export default IntegrationsSettings;
