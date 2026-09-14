import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { motion, AnimatePresence } from 'framer-motion';
import {
  ChevronDown,
  Download,
  FolderOpen,
  Globe,
  KeyRound,
  Plus,
  Puzzle,
  Search,
  ScrollText,
  Settings2,
  Trash2,
} from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { SettingsSection } from './_primitives';
import { Tooltip } from '../ui/Tooltip';
import TierBadge from '../plugins/TierBadge';
import OfficialBadge from '../plugins/OfficialBadge';
import PluginConsentModal, { ConsentSubject } from '../plugins/PluginConsentModal';
import PluginDetailOverlay from '../plugins/PluginDetailOverlay';
import PluginPanelRenderer from '../plugins/PluginPanelRenderer';
import PluginIcon from '../plugins/PluginIcon';
import InstalledBadge from '../plugins/InstalledBadge';
import { usePluginUiRegistry } from '../../plugins-ui/registry';
import {
  capabilityLines,
  compareVersions,
  IndexEntry,
  PluginInfo,
  SourceInfo,
} from '../../types/plugins';
import { Logger } from '../../utils/logger';

// Canonical bevel recipe lives in globals.css as --bevel-tile.
const TILE_BEVEL = 'var(--bevel-tile)';

const Toggle = ({ enabled, onChange }: { enabled: boolean; onChange: () => void }) => (
  <button
    type="button"
    onClick={onChange}
    className={`relative inline-flex h-6 w-11 flex-shrink-0 items-center rounded-full transition-colors ${
      enabled ? 'bg-accent' : 'bg-gray-600'
    }`}
  >
    <span
      className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
        enabled ? 'translate-x-6' : 'translate-x-1'
      }`}
    />
  </button>
);

/** Quiet icon button used in card action clusters. */
const IconAction = ({
  hint,
  onClick,
  danger = false,
  active = false,
  children,
}: {
  hint: string;
  onClick: () => void;
  danger?: boolean;
  active?: boolean;
  children: React.ReactNode;
}) => (
  <Tooltip content={hint} delay={200}>
    <button
      type="button"
      onClick={onClick}
      className={`rounded-md p-1.5 transition-colors ${
        danger
          ? 'text-textMuted hover:bg-red-500/10 hover:text-red-300'
          : active
            ? 'bg-white/[0.06] text-textPrimary'
            : 'text-textMuted hover:bg-white/[0.06] hover:text-textPrimary'
      }`}
    >
      {children}
    </button>
  </Tooltip>
);

const Chip = ({
  label,
  onClick,
  disabled = false,
  emphasis = false,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  emphasis?: boolean;
}) => (
  <button
    type="button"
    disabled={disabled}
    onClick={onClick}
    className={`rounded-lg px-3 py-1.5 text-[12px] font-medium transition-colors border ${
      disabled
        ? 'border-white/5 bg-white/[0.03] text-textMuted cursor-default'
        : emphasis
          ? 'border-accent/25 bg-accent/15 text-textPrimary hover:bg-accent/25'
          : 'border-white/10 bg-white/5 text-textSecondary hover:bg-white/10 hover:text-textPrimary'
    }`}
  >
    {label}
  </button>
);

/** Soft expand/collapse used for card details and source browsing. */
const Reveal = ({ open, children }: { open: boolean; children: React.ReactNode }) => (
  <AnimatePresence initial={false}>
    {open && (
      <motion.div
        initial={{ height: 0, opacity: 0 }}
        animate={{ height: 'auto', opacity: 1 }}
        exit={{ height: 0, opacity: 0 }}
        transition={{ duration: 0.18, ease: 'easeOut' }}
        className="overflow-hidden"
      >
        {children}
      </motion.div>
    )}
  </AnimatePresence>
);

const Hairline = () => <div className="mx-3 my-1 h-px bg-white/[0.06]" />;

const PluginsSettings = () => {
  const addToast = useAppStore((s) => s.addToast);
  // Settings panels contributed by loaded ui plugins (the in-process
  // equivalent of a process plugin's host-rendered panel).
  const uiSettingsPanels = usePluginUiRegistry((s) => s.settingsPanels);
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const [sources, setSources] = useState<SourceInfo[]>([]);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [panelOpen, setPanelOpen] = useState<string | null>(null);
  const [catalog, setCatalog] = useState<{ entry: IndexEntry; source: SourceInfo }[]>([]);
  const [search, setSearch] = useState('');
  const [tab, setTab] = useState<'discover' | 'installed' | 'sources'>('discover');
  const [detail, setDetail] = useState<{ entry: IndexEntry; source: SourceInfo } | null>(null);
  const [newSourceUrl, setNewSourceUrl] = useState('');
  const [showAddSource, setShowAddSource] = useState(false);
  const [confirmSourceUrl, setConfirmSourceUrl] = useState<string | null>(null);
  const [localDir, setLocalDir] = useState('');
  const [showDevelop, setShowDevelop] = useState(false);
  const [busy, setBusy] = useState(false);
  const [consent, setConsent] = useState<{
    subject: ConsentSubject;
    proceed: () => Promise<void>;
    abort?: () => Promise<void>;
  } | null>(null);
  const [confirmUninstall, setConfirmUninstall] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [pluginList, sourceList] = await Promise.all([
        invoke<PluginInfo[]>('plugins_list'),
        invoke<SourceInfo[]>('plugins_sources'),
      ]);
      setPlugins(pluginList);
      setSources(sourceList);
    } catch (err) {
      Logger.error('[Plugins] refresh failed:', err);
    }
  }, []);

  useEffect(() => {
    refresh();
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    const setup = async () => {
      for (const eventName of ['plugin://state-changed', 'plugin://panels-changed']) {
        const un = await listen(eventName, () => refresh());
        if (disposed) un();
        else unlisteners.push(un);
      }
    };
    setup();
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [refresh]);

  // Re-fetch every source on a timer while the marketplace is open, so a
  // plugin published to the index shows up on its own without reopening or
  // restarting (bounded by the source CDN's cache, ~minutes). `catalogTick`
  // also lets a manual refresh force it.
  const [catalogTick, setCatalogTick] = useState(0);
  useEffect(() => {
    const id = window.setInterval(() => setCatalogTick((t) => t + 1), 60_000);
    return () => window.clearInterval(id);
  }, []);

  // Aggregate every source's listings into one searchable catalog, so the
  // store shows all approved plugins up front (official source first, so it
  // wins when the same plugin id appears in more than one source).
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const ordered = [...sources].sort((a, b) => Number(b.official) - Number(a.official));
      const combined: { entry: IndexEntry; source: SourceInfo }[] = [];
      const seen = new Set<string>();
      for (const source of ordered) {
        try {
          const entries = await invoke<IndexEntry[]>('plugins_browse_source', { url: source.url });
          for (const entry of entries) {
            if (seen.has(entry.id)) continue;
            seen.add(entry.id);
            combined.push({ entry, source });
          }
        } catch {
          /* an unreachable source just contributes nothing */
        }
      }
      if (!cancelled) setCatalog(combined);
    })();
    return () => {
      cancelled = true;
    };
  }, [sources, catalogTick]);

  const filteredCatalog = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return catalog;
    return catalog.filter(
      ({ entry }) =>
        entry.name.toLowerCase().includes(q) ||
        entry.description.toLowerCase().includes(q) ||
        entry.author.name.toLowerCase().includes(q)
    );
  }, [catalog, search]);

  const fail = (err: unknown) => {
    Logger.error('[Plugins] action failed:', err);
    addToast(String(err), 'error');
  };

  const setEnabled = async (plugin: PluginInfo, enabled: boolean) => {
    const apply = async () => {
      try {
        await invoke('plugins_set_enabled', { pluginId: plugin.id, enabled });
        await refresh();
      } catch (err) {
        fail(err);
      }
    };
    // Index installs consent at install time. Local-dev folders skip that
    // step, so they get a one-time consent on first enable (per id+version).
    const devConsentKey = `plugin-consent:${plugin.id}@${plugin.version}`;
    const needsDialog =
      enabled && plugin.source === 'local-dev' && !localStorage.getItem(devConsentKey);
    if (needsDialog) {
      setConsent({
        subject: {
          name: plugin.name,
          author: plugin.author,
          version: plugin.version,
          tier: plugin.tier,
          caps: plugin.granted,
          sourceName: plugin.source === 'local-dev' ? 'a local folder' : plugin.source,
          community: false,
          action: 'Enable',
        },
        proceed: async () => {
          localStorage.setItem(devConsentKey, '1');
          await apply();
        },
      });
    } else {
      await apply();
    }
  };

  const installFromSource = async (source: SourceInfo, entry: IndexEntry) => {
    // Two-step install: download, verify, and stage first; the consent
    // dialog then shows the actual manifest capabilities; commit registers.
    // Updating a running plugin stops it first; it is re-enabled after the
    // commit (tier C runs back through its risk dialog on that re-enable).
    setBusy(true);
    try {
      const existing = plugins.find((p) => p.id === entry.id);
      const wasEnabled = existing?.enabled ?? false;
      if (wasEnabled) {
        await invoke('plugins_set_enabled', { pluginId: entry.id, enabled: false });
      }
      const preview = await invoke<{ token: string; record: PluginInfo }>(
        'plugins_begin_install',
        { sourceUrl: source.url, pluginId: entry.id }
      );
      setConsent({
        subject: {
          name: preview.record.name,
          author: preview.record.author,
          version: preview.record.version,
          tier: preview.record.tier,
          caps: preview.record.granted,
          sourceName: source.name,
          community: !source.official,
          action: 'Install',
        },
        proceed: async () => {
          try {
            const installed = await invoke<PluginInfo>('plugins_commit_install', {
              token: preview.token,
            });
            addToast(
              existing
                ? `Updated ${entry.name} to v${installed.version}`
                : `Installed ${entry.name} (disabled until you enable it)`,
              'success'
            );
            await refresh();
            if (wasEnabled) await setEnabled(installed, true);
          } catch (err) {
            fail(err);
          }
        },
        abort: async () => {
          await invoke('plugins_cancel_install', { token: preview.token }).catch(() => {});
          // A declined update restores the prior state: the still-installed
          // old version goes back to running if it was running before.
          if (wasEnabled) {
            await invoke('plugins_set_enabled', { pluginId: entry.id, enabled: true }).catch(
              () => {}
            );
          }
          await refresh();
        },
      });
    } catch (err) {
      fail(err);
    } finally {
      setBusy(false);
    }
  };

  const installLocal = async () => {
    if (!localDir.trim()) return;
    setBusy(true);
    try {
      const plugin = await invoke<PluginInfo>('plugins_install_local', {
        dir: localDir.trim(),
      });
      addToast(`Registered ${plugin.name} from folder (disabled until you enable it)`, 'success');
      setLocalDir('');
      await refresh();
    } catch (err) {
      fail(err);
    } finally {
      setBusy(false);
    }
  };

  const addSource = async () => {
    if (!confirmSourceUrl) return;
    setBusy(true);
    try {
      const source = await invoke<SourceInfo>('plugins_add_source', { url: confirmSourceUrl });
      addToast(`Added source "${source.name}" (key ${source.fingerprint})`, 'success');
      setNewSourceUrl('');
      setShowAddSource(false);
      await refresh();
    } catch (err) {
      fail(err);
    } finally {
      setConfirmSourceUrl(null);
      setBusy(false);
    }
  };

  const uninstall = async (pluginId: string) => {
    try {
      await invoke('plugins_uninstall', { pluginId });
      setConfirmUninstall(null);
      await refresh();
    } catch (err) {
      fail(err);
    }
  };

  return (
    <div className="flex flex-col gap-5">
      {/* App-store tabs + search */}
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-1 rounded-lg bg-white/[0.03] p-1">
          {(['discover', 'installed', 'sources'] as const).map((t) => (
            <button
              key={t}
              type="button"
              onClick={() => setTab(t)}
              className={`rounded-md px-3.5 py-1.5 text-[12px] font-medium capitalize transition-colors ${
                tab === t ? 'bg-white/[0.08] text-textPrimary' : 'text-textSecondary hover:text-textPrimary'
              }`}
            >
              {t === 'installed' && plugins.length > 0 ? `Installed (${plugins.length})` : t}
            </button>
          ))}
        </div>
        {tab === 'discover' && (
          <div className="relative ml-auto min-w-[200px] flex-1 sm:max-w-[320px]">
            <Search
              size={14}
              className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-textMuted"
            />
            <input
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search plugins..."
              aria-label="Search plugins"
              className="glass-input w-full rounded-lg py-2 pl-9 pr-3 text-[13px] text-textPrimary"
            />
          </div>
        )}
      </div>

      {/* Discover */}
      {tab === 'discover' &&
        (catalog.length === 0 ? (
          <div className="flex flex-col items-center py-12 text-center">
            <div
              className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl"
              style={{ background: 'rgba(165, 185, 150, 0.14)', boxShadow: TILE_BEVEL }}
            >
              <Puzzle className="h-6 w-6 text-textPrimary" strokeWidth={2} />
            </div>
            <h2 className="text-[16px] font-semibold text-textPrimary">No plugins to show</h2>
            <p className="mt-1.5 max-w-[400px] text-[13px] leading-relaxed text-textSecondary">
              Approved plugins from your sources appear here. If this stays empty, the
              sources may be unreachable, or you can add one under Sources.
            </p>
          </div>
        ) : filteredCatalog.length === 0 ? (
          <p className="py-12 text-center text-[13px] text-textSecondary">
            No plugins match your search.
          </p>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {filteredCatalog.map(({ entry, source }) => {
              const installedPlugin = plugins.find((p) => p.id === entry.id);
              const hasUpdate = Boolean(
                installedPlugin && compareVersions(entry.version, installedPlugin.version) > 0
              );
              const isCurrent = Boolean(installedPlugin) && !hasUpdate;
              return (
                <button
                  key={entry.id}
                  type="button"
                  onClick={() => setDetail({ entry, source })}
                  className="flex flex-col rounded-xl border border-white/5 bg-white/[0.03] p-3.5 text-left transition-colors hover:bg-white/[0.06]"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-1.5">
                      <span className="truncate text-[14px] font-semibold text-textPrimary">
                        {entry.name}
                      </span>
                      {entry.official && <OfficialBadge />}
                    </div>
                    <span className="mt-1 flex items-center gap-1.5 text-[11px] text-textMuted">
                      <PluginIcon
                        iconUrl={entry.icon_url}
                        official={!!entry.official}
                        author={entry.author.name}
                        tier={entry.tier}
                        sizeClass="h-5 w-5 rounded"
                        glyphSize={10}
                      />
                      <span className="truncate">by {entry.author.name}</span>
                    </span>
                  </div>
                  <div className="mt-2.5 flex items-center gap-1.5">
                    <TierBadge tier={entry.tier} />
                  </div>
                  <p
                    className="mt-2 flex-1 text-[12px] leading-relaxed text-textSecondary"
                    style={{
                      display: '-webkit-box',
                      WebkitLineClamp: 2,
                      WebkitBoxOrient: 'vertical',
                      overflow: 'hidden',
                    }}
                  >
                    {entry.description}
                  </p>
                  <div className="mt-3 flex items-center justify-between">
                    <span className="text-[11px] text-textMuted">v{entry.version}</span>
                    {isCurrent ? (
                      <InstalledBadge />
                    ) : (
                      <span className="glass-button-static px-2.5 py-1 text-[11px] font-medium text-accent">
                        {hasUpdate ? 'Update' : 'Get'}
                      </span>
                    )}
                  </div>
                </button>
              );
            })}
          </div>
        ))}

      {/* Installed */}
      {tab === 'installed' &&
        (plugins.length === 0 ? (
        <div className="flex flex-col items-center py-10 text-center">
          <div
            className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl"
            style={{ background: 'rgba(165, 185, 150, 0.14)', boxShadow: TILE_BEVEL }}
          >
            <Puzzle className="h-6 w-6 text-textPrimary" strokeWidth={2} />
          </div>
          <h2 className="text-[16px] font-semibold text-textPrimary">Nothing installed yet</h2>
          <p className="mt-1.5 max-w-[400px] text-[13px] leading-relaxed text-textSecondary">
            Plugins are separate programs StreamNook starts and supervises. Browse
            the store to add one; it only ever gets the capabilities you grant it,
            and you turn it on here when you're ready.
          </p>
          <div className="mt-5 flex gap-2">
            <Chip label="Browse the store" emphasis onClick={() => setTab('discover')} />
            <Chip label="Manage sources" onClick={() => setTab('sources')} />
          </div>
        </div>
      ) : (
        <SettingsSection
          label="Installed"
          description="The plugins you have added. Each one runs as its own process and can only do what its card lists, and you can take any permission back here."
          bare
        >
          {plugins.map((plugin) => {
            const isExpanded = expanded === plugin.id;
            const isPanelOpen = panelOpen === plugin.id;
            // A plugin has a settings panel either via the wire protocol
            // (process plugins, has_panel) or by contributing its own
            // component (ui plugins, registered while loaded).
            const UiSettingsPanel = uiSettingsPanels[plugin.id];
            const hasSettings = (plugin.has_panel || !!UiSettingsPanel) && plugin.enabled;
            // Official-ness isn't stored on the installed record; derive it
            // from the source the plugin came from (built-in source = official).
            const pluginOfficial =
              sources.find((s) => s.url === plugin.source)?.official ?? false;
            // An update is available when a source's catalog lists a newer
            // version of this installed plugin. Surfacing it here (the Installed
            // tab) means users update their own plugins without hunting for them
            // in a large Discover catalog.
            const updateMatch = catalog.find(
              (c) => c.entry.id === plugin.id && compareVersions(c.entry.version, plugin.version) > 0,
            );
            return (
              <div key={plugin.id} className="glass-panel rounded-lg p-4">
                <div className="flex items-center gap-3.5">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-[14px] font-semibold text-textPrimary">
                        {plugin.name}
                      </span>
                      {pluginOfficial && <OfficialBadge />}
                      <TierBadge tier={plugin.tier} />
                      {plugin.source === 'local-dev' && (
                        <span className="rounded border border-sky-400/20 bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-sky-300">
                          Dev
                        </span>
                      )}
                    </div>
                    <p className="mt-0.5 flex items-center gap-1.5 text-[12px] text-textSecondary">
                      {plugin.running && (
                        <span className="inline-block h-1.5 w-1.5 flex-shrink-0 rounded-full bg-emerald-400" />
                      )}
                      <span className="flex-shrink-0">
                        {plugin.running ? 'Running' : plugin.enabled ? 'Starting' : 'Off'} · v
                        {plugin.version} by
                      </span>
                      <PluginIcon
                        official={pluginOfficial}
                        author={plugin.author}
                        tier={plugin.tier}
                        sizeClass="h-5 w-5 rounded"
                        glyphSize={10}
                      />
                      <span className="truncate">{plugin.author}</span>
                    </p>
                  </div>
                  <div className="flex flex-shrink-0 items-center gap-0.5">
                    {updateMatch && (
                      <Tooltip content={`Update to v${updateMatch.entry.version}`} delay={200}>
                        <button
                          type="button"
                          onClick={() => installFromSource(updateMatch.source, updateMatch.entry)}
                          disabled={busy}
                          className="glass-button-static mr-1 flex flex-shrink-0 items-center gap-1.5 px-2.5 py-1 text-[12px] font-semibold text-amber-400 disabled:opacity-50"
                        >
                          <Download size={13} />
                          Update
                        </button>
                      </Tooltip>
                    )}
                    {hasSettings && (
                      <IconAction
                        hint="Plugin settings"
                        active={isPanelOpen}
                        onClick={() => {
                          setPanelOpen(isPanelOpen ? null : plugin.id);
                          if (!isPanelOpen) setExpanded(null);
                        }}
                      >
                        <Settings2 size={15} />
                      </IconAction>
                    )}
                    <IconAction
                      hint={isExpanded ? 'Hide details' : 'Details'}
                      active={isExpanded}
                      onClick={() => {
                        setExpanded(isExpanded ? null : plugin.id);
                        if (!isExpanded) setPanelOpen(null);
                      }}
                    >
                      <motion.span
                        animate={{ rotate: isExpanded ? 180 : 0 }}
                        transition={{ duration: 0.15 }}
                        className="block"
                      >
                        <ChevronDown size={15} />
                      </motion.span>
                    </IconAction>
                    <IconAction
                      hint="Uninstall"
                      danger
                      onClick={() => setConfirmUninstall(plugin.id)}
                    >
                      <Trash2 size={14} />
                    </IconAction>
                    <div className="ml-2">
                      <Toggle
                        enabled={plugin.enabled}
                        onChange={() => setEnabled(plugin, !plugin.enabled)}
                      />
                    </div>
                  </div>
                </div>

                <Reveal open={confirmUninstall === plugin.id}>
                  <div className="mt-3 flex items-center justify-between gap-3 rounded-lg border border-red-400/20 bg-red-500/10 px-3 py-2">
                    <span className="text-[12px] text-red-200">
                      Uninstall {plugin.name} and delete its local state?
                    </span>
                    <div className="flex flex-shrink-0 gap-2">
                      <Chip label="Cancel" onClick={() => setConfirmUninstall(null)} />
                      <button
                        type="button"
                        onClick={() => uninstall(plugin.id)}
                        className="rounded-lg border border-red-400/25 bg-red-500/20 px-3 py-1.5 text-[12px] font-medium text-red-200 transition-colors hover:bg-red-500/30"
                      >
                        Uninstall
                      </button>
                    </div>
                  </div>
                </Reveal>

                <Reveal open={isExpanded}>
                  <p className="mt-3 px-0.5 text-[12px] leading-relaxed text-textSecondary">
                    {plugin.description}
                  </p>
                  <div className="mt-3 rounded-lg bg-white/[0.02] py-1.5">
                    <div className="px-3 pb-1 pt-1.5 text-[11px] font-semibold uppercase tracking-[0.12em] text-textMuted">
                      What it can do
                    </div>
                    {capabilityLines(plugin.granted).map((line) => (
                      <div key={line.text} className="flex items-baseline gap-2 px-3 py-1">
                        <span
                          className={`h-1 w-1 flex-shrink-0 translate-y-[-2px] rounded-full ${
                            line.warning ? 'bg-red-300' : 'bg-textMuted'
                          }`}
                        />
                        <span
                          className={`text-[12px] leading-relaxed ${
                            line.warning ? 'text-red-300' : 'text-textSecondary'
                          }`}
                        >
                          {line.text}
                        </span>
                      </div>
                    ))}

                    {plugin.granted.credentials.length > 0 && (
                      <>
                        <Hairline />
                        {plugin.granted.credentials.map((kind) => {
                          const state = plugin.credential_consent[kind] ?? 'always';
                          return (
                            <div
                              key={kind}
                              className="flex items-center justify-between gap-3 px-3 py-1.5"
                            >
                              <span className="flex items-center gap-2 text-[12px] text-textSecondary">
                                <KeyRound size={12} className="flex-shrink-0 text-red-300" />
                                <span>
                                  Twitch login · {state === 'revoked' ? 'revoked' : 'allowed'}
                                </span>
                              </span>
                              <Chip
                                label={state === 'revoked' ? 'Allow' : 'Revoke'}
                                onClick={async () => {
                                  try {
                                    await invoke(
                                      state === 'revoked'
                                        ? 'plugins_reset_credential_consent'
                                        : 'plugins_revoke_credential',
                                      { pluginId: plugin.id, kind }
                                    );
                                    await refresh();
                                  } catch (err) {
                                    fail(err);
                                  }
                                }}
                              />
                            </div>
                          );
                        })}
                      </>
                    )}

                    <Hairline />
                    <div className="flex items-center justify-between gap-3 px-3 py-1.5">
                      <span className="flex items-center gap-2 text-[12px] text-textSecondary">
                        <ScrollText size={12} className="flex-shrink-0" />
                        <span className="truncate">
                          From {plugin.source === 'local-dev' ? 'a local folder' : plugin.source}
                        </span>
                      </span>
                    </div>
                  </div>
                </Reveal>

                <Reveal open={isPanelOpen}>
                  <div className="mt-3">
                    {UiSettingsPanel ? (
                      <UiSettingsPanel />
                    ) : (
                      <PluginPanelRenderer pluginId={plugin.id} />
                    )}
                  </div>
                </Reveal>
              </div>
            );
          })}
        </SettingsSection>
        ))}

      {/* Sources */}
      {tab === 'sources' && (
        <div className="flex flex-col gap-9">
      <SettingsSection
        label="Sources"
        description="Where StreamNook looks for plugins to install. Add only community sources you trust, because StreamNook does not review or host what they list."
        bare
      >
        {sources.map((source) => (
          <div key={source.url} className="glass-panel rounded-lg p-4">
            <div className="flex items-center gap-3.5">
              <div
                className="flex h-10 w-10 flex-shrink-0 items-center justify-center rounded-lg"
                style={{ background: 'rgba(150, 170, 200, 0.14)', boxShadow: TILE_BEVEL }}
              >
                <Globe size={18} strokeWidth={2} className="text-textPrimary" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate text-[14px] font-semibold text-textPrimary">
                    {source.name}
                  </span>
                  {source.official && (
                    <span className="rounded border border-emerald-400/20 bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-emerald-300">
                      Official
                    </span>
                  )}
                </div>
                <p className="mt-0.5 truncate font-mono text-[11px] text-textMuted">
                  {source.url} · key {source.fingerprint}
                </p>
              </div>
              <div className="flex flex-shrink-0 items-center gap-1.5">
                {!source.official && (
                  <IconAction
                    hint="Remove source"
                    danger
                    onClick={async () => {
                      try {
                        await invoke('plugins_remove_source', { url: source.url });
                        await refresh();
                      } catch (err) {
                        fail(err);
                      }
                    }}
                  >
                    <Trash2 size={14} />
                  </IconAction>
                )}
              </div>
            </div>
          </div>
        ))}

        {!showAddSource && sources.length === 0 && (
          <div className="glass-panel rounded-lg px-4 py-5 text-center">
            <p className="text-[12px] leading-relaxed text-textSecondary">
              No sources yet. The official StreamNook index is not live in this
              build; a community source can be added by URL.
            </p>
          </div>
        )}

        {showAddSource || sources.length > 0 ? (
          <div className="glass-panel rounded-lg p-4">
            <div className="flex items-center gap-2">
              <Plus size={14} className="flex-shrink-0 text-textMuted" />
              <input
                type="text"
                value={newSourceUrl}
                onChange={(e) => setNewSourceUrl(e.target.value)}
                placeholder="https://example.org/index.json"
                aria-label="Source index URL"
                className="glass-input flex-1 rounded-md px-3 py-1.5 text-[13px] text-textPrimary"
              />
              <Chip
                label="Add source"
                disabled={busy || !newSourceUrl.trim().startsWith('https://')}
                onClick={() => setConfirmSourceUrl(newSourceUrl.trim())}
              />
            </div>
            <Reveal open={confirmSourceUrl !== null}>
              <div className="mt-3 rounded-lg border border-white/10 bg-white/[0.04] px-3.5 py-3">
                <p className="text-[12px] leading-relaxed text-textSecondary">
                  Add a community plugin source? StreamNook doesn't review or host
                  what community sources list, so add ones you trust. The source's
                  signing key is verified and pinned when it's added; future updates
                  must be signed with the same key.
                </p>
                <p className="mt-1.5 truncate font-mono text-[11px] text-amber-200/70">
                  {confirmSourceUrl}
                </p>
                <div className="mt-2.5 flex justify-end gap-2">
                  <Chip label="Cancel" onClick={() => setConfirmSourceUrl(null)} />
                  <button
                    type="button"
                    disabled={busy}
                    onClick={addSource}
                    className="rounded-lg border border-amber-400/25 bg-amber-500/20 px-3 py-1.5 text-[12px] font-medium text-amber-200 transition-colors hover:bg-amber-500/30"
                  >
                    Add source
                  </button>
                </div>
              </div>
            </Reveal>
          </div>
        ) : (
          <div className="flex justify-center pt-1">
            <Chip label="Add a community source" onClick={() => setShowAddSource(true)} />
          </div>
        )}
      </SettingsSection>

      {/* Develop */}
      <SettingsSection
        label="Develop"
        description="Load a plugin you are building straight from its folder (the one containing plugin.toml). It shows as Dev, skips signature checks, and still asks for the same permissions as a store plugin."
        bare
      >
        {showDevelop || localDir ? (
          <div className="glass-panel rounded-lg p-4">
            <div className="flex items-center gap-2">
              <FolderOpen size={14} className="flex-shrink-0 text-textMuted" />
              <input
                type="text"
                value={localDir}
                onChange={(e) => setLocalDir(e.target.value)}
                placeholder="C:\path\to\my-plugin"
                aria-label="Plugin folder path"
                className="glass-input flex-1 rounded-md px-3 py-1.5 font-mono text-[13px] text-textPrimary"
              />
              <Chip
                label="Register"
                emphasis
                disabled={busy || !localDir.trim()}
                onClick={installLocal}
              />
            </div>
          </div>
        ) : (
          <div className="flex justify-center pt-1">
            <Chip label="Register a plugin folder" onClick={() => setShowDevelop(true)} />
          </div>
        )}
      </SettingsSection>
        </div>
      )}

      <PluginDetailOverlay
        entry={detail?.entry ?? null}
        sourceName={detail?.source.name ?? ''}
        installed={plugins.find((p) => p.id === detail?.entry.id)}
        busy={busy}
        onClose={() => setDetail(null)}
        onInstall={(entry) => {
          const source = detail?.source;
          setDetail(null);
          if (source) installFromSource(source, entry);
        }}
      />

      <PluginConsentModal
        subject={consent?.subject ?? null}
        onCancel={async () => {
          const abort = consent?.abort;
          setConsent(null);
          if (abort) await abort();
        }}
        onConfirm={async () => {
          const proceed = consent?.proceed;
          setConsent(null);
          if (proceed) await proceed();
        }}
      />
    </div>
  );
};

export default PluginsSettings;
