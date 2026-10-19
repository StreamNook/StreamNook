import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { SettingsSection, SettingsRow, SegmentedSelect } from './_primitives';
import { SubControl, SubControls } from '../plugins/settingsPageKit';
import { MOD_LOG_CATEGORIES, MOD_LOG_STYLES, highlightContainerStyle } from '../../utils/modLogCategories';
import { Tooltip } from '../ui/Tooltip';
import { connectModRoomConsent, clearModeratedCache, loadModeratedChannelIds } from '../../services/modRoomService';
import { useSettingDefault, useSettingReset } from './settingReset';
import { ModHoverPreview, ModLinePreview, ModSliderPreview } from './ChatPreview';
import { HoverTimeoutsEditor, ModSliderStepsEditor } from './ModSliderSteps';

const Toggle = ({ enabled, onChange }: { enabled: boolean; onChange: () => void }) => (
  <button
    onClick={onChange}
    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors flex-shrink-0 ${
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

/** What sits at the left edge of a message a moderator can act on. */
type ModEdge = 'slider' | 'buttons' | 'off';

/** The edge the stored keys draw: the line buttons win, then the slider, as
 *  ChatMessage does. */
function edgeOf(slider: unknown, line: unknown): ModEdge {
  if (line === true) return 'buttons';
  return slider !== false ? 'slider' : 'off';
}

const hoverOf = (style: unknown) => style !== 'none';

/** One plain sentence per edge, so the row says what the picked one does. */
const EDGE_DESCRIPTION: Record<ModEdge, string> = {
  slider: 'Press the dotted handle on a message\'s left edge and drag right: Delete, then longer timeouts, then Ban, which waits for a short pause. Let go to apply. Drag back, move off the message or press Esc to cancel.',
  buttons: 'Delete, a timeout and Ban as small buttons at the start of a message, one click each. Scroll over the timeout to make it longer or shorter.',
  off: 'Nothing on the left edge. Use the hover buttons below, or the keyboard.',
};

const ModerationSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const mod = settings.moderation ?? {};
  const resetFor = useSettingReset();

  // Mod-room scoped consent: which account it belongs to, and the two actions.
  const [modRoomLogin, setModRoomLogin] = useState<string | null>(null);
  const [modRoomBusy, setModRoomBusy] = useState(false);
  useEffect(() => {
    invoke<{ connected: boolean; login: string | null }>('modroom_status')
      .then((s) => setModRoomLogin(s.connected ? s.login : null))
      .catch(() => {});
  }, []);
  const handleModRoomConnect = async () => {
    setModRoomBusy(true);
    try {
      const login = await connectModRoomConsent();
      if (login) setModRoomLogin(login);
    } catch {
      // cancelled / failed; leave the button
    } finally {
      setModRoomBusy(false);
    }
  };
  const handleModRoomDisconnect = async () => {
    setModRoomBusy(true);
    try {
      await invoke('modroom_disconnect');
      setModRoomLogin(null);
      clearModeratedCache();
      void loadModeratedChannelIds(true);
    } catch {
      // nothing to disconnect
    } finally {
      setModRoomBusy(false);
    }
  };

  const setMod = (patch: Partial<typeof mod>) =>
    updateSettings({ ...settings, moderation: { ...mod, ...patch } });

  // How a moderator acts on a message, as two choices. Message edge is what
  // sits at a message's left edge: the slider, the line buttons or nothing.
  // Hover buttons are Ban, Delete and a timeout in the corner of the hovered
  // message. They are stored as three keys (mod_slider, mod_line_buttons and
  // mod_action_style); every write sets all three, so the two rows always say
  // what chat does. Lives in chat_design because chat reads it per row.
  const cd = settings.chat_design;
  const modEdge = edgeOf(cd?.mod_slider, cd?.mod_line_buttons);
  const hoverButtons = hoverOf(cd?.mod_action_style);
  const writeModGestures = (edge: ModEdge, hover: boolean) =>
    updateSettings({
      ...settings,
      chat_design: {
        ...settings.chat_design!,
        mod_slider: edge === 'slider',
        mod_line_buttons: edge === 'buttons',
        mod_action_style: hover ? 'buttons' : 'none',
      },
    });
  // The rows' arrows compare what they show with a fresh install's.
  const defaultStyle = useSettingDefault('chat_design.mod_action_style');
  const defaultEdge = edgeOf(
    useSettingDefault('chat_design.mod_slider'),
    useSettingDefault('chat_design.mod_line_buttons'),
  );
  const defaultHover = hoverOf(defaultStyle);
  const defaultsLoaded = defaultStyle !== undefined;

  return (
    <div className="space-y-8">
      <SettingsSection
        id="settings-section-streamer-mode"
        label="Streamer Mode"
        description="While you are live, hide what should not be on stream: viewer counts in the chat header, link previews are muted, rows from restricted (low-trust) users are hidden, and highlight sounds stay silent. Auto watches for OBS, Streamlabs, XSplit, Twitch Studio and vMix."
      >
        <SettingsRow
          title="Activation"
          onReset={resetFor(['streamer_mode.mode', 'off'])}
          description="Streamer mode off, always on, or automatic while broadcasting software is running."
        >
          <SegmentedSelect<'off' | 'on' | 'auto'>
            value={settings.streamer_mode?.mode ?? 'off'}
            onChange={(mode) => updateSettings({ ...settings, streamer_mode: { ...settings.streamer_mode, mode } })}
            options={[
              { value: 'off', label: 'Off' },
              { value: 'auto', label: 'Auto' },
              { value: 'on', label: 'On' },
            ]}
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Moderation Actions"
        description="Ban, time out or delete straight from a message. Only in channels where you are a mod or the broadcaster, and never against yourself, the channel owner, or other moderators unless it is your channel."
      >
        <SettingsRow
          title="Message edge"
          onReset={
            defaultsLoaded && modEdge !== defaultEdge ? () => writeModGestures(defaultEdge, hoverButtons) : undefined
          }
          description={EDGE_DESCRIPTION[modEdge]}
          help="The slider and the line buttons appear on messages from other chatters. On your own messages, the channel owner's, and other moderators' (unless it is your channel) there is nothing to ban or time out, so they stay away. Each slider step is the same distance at every chat width, set by Length; a narrow chat squeezes them to fit. Every timeout and ban has an Undo in its notice. Chat holds still while your pointer is on a message you can act on, so it cannot scroll away mid-aim; it catches up when you move off. Timeouts follow each platform: Kick rounds up to whole minutes and stops at a week, YouTube has one 5 minute timeout."
          control={
            <SegmentedSelect<ModEdge>
              value={modEdge}
              onChange={(edge) => writeModGestures(edge, hoverButtons)}
              options={[
                { value: 'slider', label: 'Slider' },
                { value: 'buttons', label: 'Line buttons' },
                { value: 'off', label: 'Off' },
              ]}
            />
          }
        >
          {modEdge === 'slider' && (
            <>
              <ModSliderPreview stored={cd} />
              <SubControls>
                <SubControl
                  title="Style"
                  onReset={resetFor('chat_design.mod_slider_style')}
                  control={
                    <SegmentedSelect<'preview' | 'rail' | 'capsule'>
                      value={cd?.mod_slider_style ?? 'preview'}
                      onChange={(v) =>
                        updateSettings({ ...settings, chat_design: { ...settings.chat_design!, mod_slider_style: v } })
                      }
                      options={[
                        { value: 'preview', label: 'Preview' },
                        { value: 'rail', label: 'Rail' },
                        { value: 'capsule', label: 'Capsule' },
                      ]}
                    />
                  }
                />
                <SubControl
                  title="Length"
                  onReset={resetFor('chat_design.mod_slider_length')}
                  control={
                    <SegmentedSelect<'short' | 'medium' | 'long'>
                      value={cd?.mod_slider_length ?? 'medium'}
                      onChange={(v) =>
                        updateSettings({ ...settings, chat_design: { ...settings.chat_design!, mod_slider_length: v } })
                      }
                      options={[
                        { value: 'short', label: 'Short' },
                        { value: 'medium', label: 'Medium' },
                        { value: 'long', label: 'Long' },
                      ]}
                    />
                  }
                />
                <SubControl
                  title="Steps"
                  stacked
                  onReset={resetFor('chat_design.mod_slider_delete', 'chat_design.mod_slider_timeouts', 'chat_design.mod_slider_ban')}
                  control={<ModSliderStepsEditor />}
                />
              </SubControls>
            </>
          )}
          {modEdge === 'buttons' && (
            <>
              <ModLinePreview stored={cd} />
              <SubControls>
                <SubControl
                  title="Show"
                  onReset={resetFor('chat_design.mod_line_show')}
                  control={
                    <SegmentedSelect<'hover' | 'always'>
                      value={cd?.mod_line_show === 'always' ? 'always' : 'hover'}
                      onChange={(v) =>
                        updateSettings({ ...settings, chat_design: { ...settings.chat_design!, mod_line_show: v } })
                      }
                      options={[
                        { value: 'hover', label: 'On hover' },
                        { value: 'always', label: 'Always' },
                      ]}
                    />
                  }
                />
                <SubControl
                  title="Buttons"
                  stacked
                  onReset={resetFor('chat_design.mod_line_delete', 'chat_design.mod_line_timeouts', 'chat_design.mod_line_ban')}
                  control={<ModSliderStepsEditor list="line" />}
                />
              </SubControls>
            </>
          )}
        </SettingsRow>
        <SettingsRow
          title="Hover buttons"
          onReset={
            defaultsLoaded && hoverButtons !== defaultHover ? () => writeModGestures(modEdge, defaultHover) : undefined
          }
          description="Ban, Delete and a 10 minute timeout in the corner of the message under your pointer, with the other lengths one click away. Copy and Pin are always there."
          control={<Toggle enabled={hoverButtons} onChange={() => writeModGestures(modEdge, !hoverButtons)} />}
        >
          {hoverButtons && (
            <>
              <ModHoverPreview stored={cd} />
              <SubControls>
                <SubControl
                  title="Timeout lengths"
                  stacked
                  onReset={resetFor('moderation.timeout_presets')}
                  control={<HoverTimeoutsEditor />}
                />
              </SubControls>
            </>
          )}
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        id="settings-section-mod-reasons"
        label="Reasons"
        description="What gets recorded against a ban or timeout. Twitch shows these in the channel's mod view."
      >
        <SettingsRow
          title="/nuke reason"
          description="Written against every ban and timeout a /nuke issues. Leave empty to record just “/nuke”."
        >
          <input
            type="text"
            value={mod.nuke_reason ?? ''}
            onChange={(e) => setMod({ nuke_reason: e.target.value })}
            placeholder="/nuke"
            maxLength={140}
            className="glass-input w-full px-3 py-2 text-sm text-textPrimary placeholder-textSecondary"
          />
        </SettingsRow>
        <SettingsRow
          title="Saved reasons"
          description="Offered when you ban or time someone out from their card or a message. One per line; the first is filled in for you."
        >
          <textarea
            rows={4}
            value={(mod.saved_ban_reasons ?? []).join('\n')}
            onChange={(e) =>
              setMod({
                saved_ban_reasons: e.target.value
                  .split('\n')
                  .map((s) => s.trim())
                  .filter(Boolean),
              })
            }
            placeholder={'Spam\nHate speech\nBackseating\nBan evasion'}
            className="glass-input w-full px-3 py-2 text-sm text-textPrimary placeholder-textSecondary resize-y scrollbar-thin"
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Mod Logs"
        description="A running list of timeouts, bans, and deletions in the channel you are watching, so you can see what the mod team is doing."
      >
        <SettingsRow
          title="Log panel"
          onReset={resetFor(['show_mod_logs', false])}
          description="Shows the mod log beside chat: a panel listing recent timeouts, bans, and deleted messages as they happen."
          control={
            <Toggle
              enabled={settings.show_mod_logs ?? false}
              onChange={() =>
                updateSettings({ ...settings, show_mod_logs: !(settings.show_mod_logs ?? false) })
              }
            />
          }
        />
      </SettingsSection>

      <SettingsSection
        label="Mod Rooms"
        description="Private, encrypted chat rooms for the mod teams of channels you moderate, unlocked by a separate one-time Twitch consent that only proves which channels you moderate and cannot act on your account."
      >
        <SettingsRow
          title="Connection"
          description={
            modRoomLogin
              ? `Connected as ${modRoomLogin}. Rooms verify mod status and open as this account. Disconnect to switch accounts or revoke access.`
              : 'Not connected. Grant the one-time consent to unlock rooms for the channels you moderate.'
          }
          control={
            modRoomLogin ? (
              <button
                onClick={handleModRoomDisconnect}
                disabled={modRoomBusy}
                className="px-3 py-1.5 text-xs font-medium text-textSecondary transition-colors hover:text-accent disabled:opacity-50"
              >
                Disconnect
              </button>
            ) : (
              <button
                onClick={handleModRoomConnect}
                disabled={modRoomBusy}
                className="px-3 py-1.5 text-xs font-medium text-textSecondary transition-colors hover:text-accent disabled:opacity-50"
              >
                {modRoomBusy ? 'Opening browser...' : 'Connect'}
              </button>
            )
          }
        />
      </SettingsSection>

      <SettingsSection
        label="Removed Messages"
        description="How timeouts, bans, and deletions show up inside the chat itself. By default a removed message stays in place with a strikethrough."
      >
        <SettingsRow
          title="Action notices"
          onReset={resetFor(['moderation.show_mod_messages', false])}
          description="Announces mod actions inline: an extra row in chat when a mod times someone out, bans, or deletes a message. Stacks on top of the strikethrough you already see."
          control={
            <Toggle
              enabled={mod.show_mod_messages ?? false}
              onChange={() => setMod({ show_mod_messages: !(mod.show_mod_messages ?? false) })}
            />
          }
        />
        <SettingsRow
          title="Hide strikethrough"
          onReset={resetFor(['moderation.ignore_clear_chat', false])}
          description="Banned, timed-out, and deleted messages stay exactly as they were, with no line through them."
          control={
            <Toggle
              enabled={mod.ignore_clear_chat ?? false}
              onChange={() => setMod({ ignore_clear_chat: !(mod.ignore_clear_chat ?? false) })}
            />
          }
        />
      </SettingsSection>

      <SettingsSection
        label="Log Highlights"
        description="Color-code mod-log entries by severity. Choose how the highlight shows, then customize any category's color."
      >
        <SettingsRow
          title="Style"
          onReset={resetFor(['moderation.mod_log_highlight_style', 'box'])}
          description="How each entry is emphasized by severity. The previews use a sample event."
        >
          <div className="grid grid-cols-3 gap-2">
            {MOD_LOG_STYLES.map(({ key, label }) => {
              const active = (mod.mod_log_highlight_style ?? 'box') === key;
              const sample = '#e5484d'; // representative (ban) color
              return (
                <button
                  key={key}
                  onClick={() => setMod({ mod_log_highlight_style: key })}
                  className={`flex flex-col items-center gap-1.5 rounded-lg p-1.5 transition-all ${
                    active ? 'ring-1 ring-white/20 bg-glass/40' : 'opacity-60 hover:opacity-100'
                  }`}
                >
                  <div
                    className="w-full rounded-md px-2 py-1.5 bg-secondary/60 border border-borderSubtle text-left"
                    style={highlightContainerStyle(key, sample)}
                  >
                    <div className="flex items-center gap-1.5">
                      <span
                        className="h-2 w-2 rounded-full flex-shrink-0"
                        style={{ backgroundColor: sample }}
                      />
                      <span
                        className={`text-[10px] font-semibold truncate ${key === 'box' ? 'text-text' : ''}`}
                        style={key === 'box' ? undefined : { color: sample }}
                      >
                        Banned
                      </span>
                    </div>
                  </div>
                  <span
                    className={`text-[11px] ${active ? 'text-text font-medium' : 'text-textSecondary'}`}
                  >
                    {label}
                  </span>
                </button>
              );
            })}
          </div>
        </SettingsRow>
        {MOD_LOG_CATEGORIES.map((c) => {
          const current = mod.mod_log_colors?.[c.key] || c.defaultColor;
          const overridden = !!mod.mod_log_colors?.[c.key];
          return (
            <SettingsRow
              key={c.key}
              title={c.label}
              onReset={
                overridden
                  ? () => {
                      const next = { ...(mod.mod_log_colors ?? {}) };
                      delete next[c.key];
                      setMod({ mod_log_colors: next });
                    }
                  : undefined
              }
            >
              <div className="flex items-center gap-2">
                <Tooltip content={`${c.label} color`}>
                <input
                  type="color"
                  value={current}
                  onChange={(e) =>
                    setMod({
                      mod_log_colors: { ...(mod.mod_log_colors ?? {}), [c.key]: e.target.value },
                    })
                  }
                  className="h-7 w-10 rounded cursor-pointer bg-transparent border border-borderSubtle"
                />
                </Tooltip>
              </div>
            </SettingsRow>
          );
        })}
      </SettingsSection>

      <SettingsSection
        label="Mass Actions"
        description="Commands you type in any chat box to act on many messages at once. They only work in channels where you are a mod; elsewhere they do nothing."
      >
        <SettingsRow
          title="/nuke"
          description="Bans, times out, or deletes every recent message that matches a word or /regex/flags, typed as /nuke <pattern> <action> <past[:future]>."
          help="Pattern is the text to match. Action is delete, ban, or a duration like 10m. Past is how far back to look; add :future to keep catching new matches for that long after."
        >
          <div className="space-y-1.5 text-[12px] text-textSecondary leading-relaxed">
            <div>
              <code className="rounded bg-glass/50 px-1.5 py-0.5 font-mono text-textPrimary">
                /nuke spam ban 5m:1m
              </code>
              <span className="ml-2">Ban anyone whose recent 5 minutes contains &quot;spam&quot;, keep banning matches for the next minute.</span>
            </div>
            <div>
              <code className="rounded bg-glass/50 px-1.5 py-0.5 font-mono text-textPrimary">
                /nuke /raid|follow.?for.?follow/i 10m 10m
              </code>
              <span className="ml-2">10 minute timeout for raid/follow4follow patterns going back 10 minutes.</span>
            </div>
            <div>
              <code className="rounded bg-glass/50 px-1.5 py-0.5 font-mono text-textPrimary">
                /nuke bigotry delete 1h
              </code>
              <span className="ml-2">Delete every matching message in the last hour.</span>
            </div>
          </div>
        </SettingsRow>
        <SettingsRow
          title="/undo"
          description="Reverses the most recent /nuke in this channel. Bans and timeouts are lifted; deleted messages stay gone because Twitch cannot restore them."
        />
      </SettingsSection>
    </div>
  );
};

export default ModerationSettings;
