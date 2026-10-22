// Image Uploads: the emote menu's Images section, paste upload (desktop) and
// where both send pictures. Own section (it is a feature, not a composer
// detail), with host tiles like ShareX's destinations and a one-click test so
// nobody has to discover a broken host mid-conversation. The host list and
// the test come from Rust (services/image_upload.rs), which owns the request.

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, ImageUp, Loader2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { IS_MOBILE } from '../../utils/platform';
import type { UploadHost } from '../../types';
import { SettingsSection, SettingsRow, SettingsSubGroup } from './_primitives';
import { Toggle } from '../ui/Toggle';
import { useSettingReset } from './settingReset';
import { Tooltip } from '../ui/Tooltip';

const CUSTOM_HOST_ID = 'custom';
const DEFAULT_HOST_ID = 'nuuls';
const NUULS_URL = 'https://i.nuuls.com/upload';

const ImageUploadSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const resetFor = useSettingReset();
  const up = settings.chat_input?.image_uploader ?? {};
  const pasteOn = up.enabled ?? false;
  const tabOn = up.show_in_menu ?? true;
  const presetId = up.preset ?? (up.url && up.url !== NUULS_URL ? CUSTOM_HOST_ID : DEFAULT_HOST_ID);
  const setUp = (patch: Partial<typeof up>) =>
    updateSettings({
      ...settings,
      chat_input: { ...settings.chat_input, image_uploader: { ...up, ...patch } },
    });

  const [hosts, setHosts] = useState<UploadHost[]>([]);
  useEffect(() => {
    void invoke<UploadHost[]>('image_upload_hosts').then(setHosts, () => {});
  }, []);

  const [test, setTest] = useState<{ state: 'idle' | 'busy' | 'ok' | 'fail'; link?: string; error?: string }>({ state: 'idle' });
  const runTest = () => {
    setTest({ state: 'busy' });
    void invoke<string>('test_image_upload').then(
      (link) => setTest({ state: 'ok', link }),
      (err) => setTest({ state: 'fail', error: typeof err === 'string' ? err : 'Upload failed' }),
    );
  };

  // The phones pick from the preset hosts; typing a multipart field name is a
  // PC task, so "My own host" is offered on the desktop only.
  const tiles: (UploadHost | null)[] = IS_MOBILE ? hosts : [...hosts, null];

  return (
    <SettingsSection
      id="settings-section-image-uploads"
      label="Image Uploads"
      description="Send a picture to chat from the emote menu's Images tab. It goes to the host you pick here, and its link lands in your message."
    >
      <SettingsRow
        title="Images tab"
        onReset={resetFor(['chat_input.image_uploader.show_in_menu', true])}
        description="The Images tab in the emote menu, for choosing a picture to send."
        help="The first time you send a picture to a host, StreamNook asks once. Links from every host here show as pictures in StreamNook chat."
        control={<Toggle enabled={tabOn} onChange={() => setUp({ show_in_menu: !tabOn })} />}
      />
      {!IS_MOBILE && (
        <SettingsRow
          title="Paste upload"
          onReset={resetFor(['chat_input.image_uploader.enabled', false])}
          description="Paste a screenshot into the message box to upload it. Off by default, since a paste can be an accident."
          help="Only images pasted into the chat box are sent, and only while this is on. Text pastes are never touched."
          control={<Toggle enabled={pasteOn} onChange={() => setUp({ enabled: !pasteOn })} />}
        />
      )}
      <SettingsRow
        title="Where images go"
        onReset={resetFor(['chat_input.image_uploader.preset', DEFAULT_HOST_ID])}
        description="The host your pictures are sent to. Anyone with a link can see the picture."
      >
        <div className={`grid w-full gap-2 ${IS_MOBILE ? 'grid-cols-2' : 'grid-cols-2 xl:grid-cols-3'}`}>
          {tiles.map((h) => {
            const id = h ? h.id : CUSTOM_HOST_ID;
            const active = presetId === id;
            return (
              <button
                key={id}
                type="button"
                onClick={() => setUp({ preset: id })}
                aria-pressed={active}
                className={`glass-tile flex min-h-[64px] flex-col items-start gap-0.5 px-3 py-2 text-left transition-[box-shadow,background-color] ${
                  active ? 'shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--color-accent)_55%,transparent),inset_0_1px_0_0_rgba(255,255,255,0.10)]' : 'hover:bg-white/[0.03]'
                }`}
              >
                <span className="flex w-full items-center justify-between gap-2">
                  <span className={`text-[13px] font-medium ${active ? 'text-textPrimary' : 'text-textPrimary/90'}`}>{h ? h.label : 'My own host'}</span>
                  {active && <Check size={13} className="flex-shrink-0 text-accent" />}
                </span>
                <span className="text-[11px] leading-snug text-textSecondary">
                  {h ? h.note : 'Any host that takes a multipart upload and answers with the link.'}
                </span>
                {h && <span className="text-[10px] text-textMuted">up to {h.max_mb} MB</span>}
              </button>
            );
          })}
        </div>
      </SettingsRow>
      {!IS_MOBILE && presetId === CUSTOM_HOST_ID && (
        <SettingsSubGroup>
          <SettingsRow
            title="Custom host"
            description="What most self-hosted uploaders ask for. If the host answers with the link as plain text, leave the last box empty."
          >
            <div className="flex w-full flex-col gap-2">
              <label className="flex flex-col gap-1 text-[11px] text-textSecondary">
                Upload address (https)
                <input
                  type="url"
                  value={up.url ?? ''}
                  onChange={(e) => setUp({ url: e.target.value })}
                  placeholder="https://your-host.example/upload"
                  className="glass-input w-full px-2.5 py-1.5 text-sm text-textPrimary"
                  spellCheck={false}
                />
              </label>
              <div className="flex flex-wrap gap-2">
                <label className="flex w-40 flex-col gap-1 text-[11px] text-textSecondary">
                  File field name
                  <input
                    type="text"
                    value={up.form_field ?? 'file'}
                    onChange={(e) => setUp({ form_field: e.target.value })}
                    placeholder="file"
                    className="glass-input w-full px-2.5 py-1.5 text-sm text-textPrimary"
                    spellCheck={false}
                  />
                </label>
                <label className="flex min-w-[200px] flex-1 flex-col gap-1 text-[11px] text-textSecondary">
                  Extra form fields (optional)
                  <input
                    type="text"
                    value={up.extra_fields ?? ''}
                    onChange={(e) => setUp({ extra_fields: e.target.value })}
                    placeholder="for example reqtype=fileupload&time=72h"
                    className="glass-input w-full px-2.5 py-1.5 text-sm text-textPrimary"
                    spellCheck={false}
                  />
                </label>
                <Tooltip content="Paste the Image link from a host's uploader setup as it is. Empty means the reply is the link itself." describe>
                  <label
                    className="flex min-w-[200px] flex-1 flex-col gap-1 text-[11px] text-textSecondary"
                  >
                    Where the link is in the reply (optional)
                    <input
                      type="text"
                      value={up.response_path ?? ''}
                      onChange={(e) => setUp({ response_path: e.target.value })}
                      placeholder="for example {url}, {link}{ext} or data.link"
                      className="glass-input w-full px-2.5 py-1.5 text-sm text-textPrimary"
                      spellCheck={false}
                    />
                  </label>
                </Tooltip>
              </div>
            </div>
          </SettingsRow>
        </SettingsSubGroup>
      )}
      <SettingsRow
        title="Test upload"
        description="Sends a 1-pixel test image to the host you picked and shows the link it came back with."
        control={
          <button
            type="button"
            onClick={runTest}
            disabled={test.state === 'busy'}
            className="glass-button inline-flex items-center gap-2 rounded-lg px-3 py-1.5 text-sm font-medium text-textPrimary disabled:opacity-60"
          >
            {test.state === 'busy' ? <Loader2 size={14} className="animate-spin" /> : <ImageUp size={14} />}
            Send a test image
          </button>
        }
      >
        {test.state === 'ok' && test.link && (
          <div className="flex items-center gap-2 text-xs">
            <Check size={13} className="text-emerald-400" />
            <span className="text-textSecondary">Worked. The host answered:</span>
            <code className="truncate text-textPrimary">{test.link}</code>
          </div>
        )}
        {test.state === 'fail' && <div className="text-xs text-error">{test.error}</div>}
      </SettingsRow>
    </SettingsSection>
  );
};

export default ImageUploadSettings;
