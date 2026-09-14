// Image Uploads: paste a picture into chat, it goes to a host you picked
// once, and the link lands in your message. Own section (it is a feature,
// not a composer detail), with host tiles like ShareX's destinations and a
// one-click test so nobody has to discover a broken host mid-conversation.

import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, ImageUp, Loader2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { SettingsSection, SettingsRow, SettingsSubGroup } from './_primitives';
import { Toggle } from '../ui/Toggle';
import {
  CUSTOM_HOST_ID,
  DEFAULT_HOST_ID,
  UPLOAD_HOST_PRESETS,
  encodeExtraFields,
  resolveUploadTarget,
} from '../../utils/imageUploadHosts';

/** A 1x1 transparent PNG, enough to prove the host round trip. */
function testPngBytes(): Uint8Array {
  const b64 = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

const ImageUploadSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const up = settings.chat_input?.image_uploader ?? {};
  const enabled = up.enabled ?? false;
  const presetId = up.preset ?? (up.url && up.url !== UPLOAD_HOST_PRESETS[0].url ? CUSTOM_HOST_ID : DEFAULT_HOST_ID);
  const setUp = (patch: Partial<typeof up>) =>
    updateSettings({
      ...settings,
      chat_input: { ...settings.chat_input, image_uploader: { ...up, ...patch } },
    });

  const [test, setTest] = useState<{ state: 'idle' | 'busy' | 'ok' | 'fail'; link?: string; error?: string }>({ state: 'idle' });
  const runTest = async () => {
    const target = resolveUploadTarget({ ...up, preset: presetId });
    setTest({ state: 'busy' });
    try {
      const link = await invoke<string>('upload_image', testPngBytes(), {
        headers: {
          'x-upload-url': target.url,
          'x-form-field': target.formField,
          'x-extra-fields': encodeExtraFields(target.extraFields),
          'x-response-path': target.responsePath,
          'x-filename': 'streamnook-test.png',
          'x-mime': 'image/png',
        },
      });
      setTest({ state: 'ok', link });
    } catch (err) {
      setTest({ state: 'fail', error: typeof err === 'string' ? err : 'Upload failed' });
    }
  };

  return (
    <SettingsSection
      id="settings-section-image-uploads"
      label="Image Uploads"
      description="Twitch chat can't carry pictures, so StreamNook can send one to an image host for you and drop the link into your message. Copy a screenshot, press Ctrl+V in the chat box, done."
    >
      <SettingsRow
        title="Paste images to upload"
        description="Only images pasted into the chat box are sent, and only while this is on. Text pastes are never touched."
        help="Off by default because the picture leaves your PC for the host you choose below. Links from every host here render as image cards in StreamNook chat."
        control={<Toggle enabled={enabled} onChange={() => setUp({ enabled: !enabled })} />}
      />
      {enabled && (
        <SettingsSubGroup>
          <div className="px-4 pb-2 pt-2">
            <div className="mb-2 text-[11px] font-semibold uppercase tracking-[0.12em] text-textMuted">Where images go</div>
            <div className="grid grid-cols-2 gap-2 xl:grid-cols-3">
              {[...UPLOAD_HOST_PRESETS, null].map((p) => {
                const id = p ? p.id : CUSTOM_HOST_ID;
                const active = presetId === id;
                return (
                  <button
                    key={id}
                    type="button"
                    onClick={() => setUp(p ? { preset: p.id, url: p.url, form_field: p.formField, extra_fields: encodeExtraFields(p.extraFields ?? {}), response_path: p.responsePath ?? '' } : { preset: CUSTOM_HOST_ID })}
                    aria-pressed={active}
                    className={`glass-tile flex min-h-[64px] flex-col items-start gap-0.5 px-3 py-2 text-left transition-[box-shadow,background-color] ${
                      active ? 'shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--color-accent)_55%,transparent),inset_0_1px_0_0_rgba(255,255,255,0.10)]' : 'hover:bg-white/[0.03]'
                    }`}
                  >
                    <span className="flex w-full items-center justify-between gap-2">
                      <span className={`text-[13px] font-medium ${active ? 'text-textPrimary' : 'text-textPrimary/90'}`}>{p ? p.label : 'My own host'}</span>
                      {active && <Check size={13} className="flex-shrink-0 text-accent" />}
                    </span>
                    <span className="text-[11px] leading-snug text-textSecondary">
                      {p ? p.note : 'Any host that takes a multipart upload and answers with the link.'}
                    </span>
                    {p && <span className="text-[10px] text-textMuted">up to {p.maxMb} MB</span>}
                  </button>
                );
              })}
            </div>
          </div>
          {presetId === CUSTOM_HOST_ID && (
            <SettingsRow
              title="Host details"
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
                  <label className="flex min-w-[200px] flex-1 flex-col gap-1 text-[11px] text-textSecondary">
                    Where the link is in a JSON reply (optional)
                    <input
                      type="text"
                      value={up.response_path ?? ''}
                      onChange={(e) => setUp({ response_path: e.target.value })}
                      placeholder="for example data.link"
                      className="glass-input w-full px-2.5 py-1.5 text-sm text-textPrimary"
                      spellCheck={false}
                    />
                  </label>
                </div>
              </div>
            </SettingsRow>
          )}
          <SettingsRow
            title="Try it"
            description="Sends a 1-pixel test image to the host you picked and shows the link it came back with."
            control={
              <button
                type="button"
                onClick={() => void runTest()}
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
        </SettingsSubGroup>
      )}
    </SettingsSection>
  );
};

export default ImageUploadSettings;
