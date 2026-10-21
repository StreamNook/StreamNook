// The emote menu's Images section: choose a picture, it goes to the image
// host picked in Settings, and its link lands in the message box. Below,
// the pictures uploaded recently, one tap to use again. It presents Rust's
// panel model (image_upload_panel); the upload itself, the one-time host
// question and its retry live in services/imageUploadService.

import { useEffect, useRef, useState } from 'react';
import { ImageUp, Loader2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { IS_MOBILE } from '../../utils/platform';
import type { ImageUploadPanel } from '../../types';
import {
  imageUploadPanel,
  pickImagePath,
  uploadWithConsent,
  type UploadSource,
} from '../../services/imageUploadService';

export default function ImageUploadSection({ onInsert }: { onInsert: (text: string) => void }) {
  const [panel, setPanel] = useState<ImageUploadPanel | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hidden, setHidden] = useState<Set<string>>(() => new Set());
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    void imageUploadPanel().then(setPanel, () => {});
  }, []);

  const run = (source: UploadSource) => {
    setError(null);
    void uploadWithConsent(source, () => setBusy(true)).then((out) => {
      setBusy(false);
      if (out.status === 'done') {
        onInsert(out.image.url);
        void imageUploadPanel().then(setPanel, () => {});
      } else if (out.status === 'failed') {
        setError(out.error);
        if (!IS_MOBILE) useAppStore.getState().addToast(out.error, 'error');
      }
    });
  };

  const choose = () => {
    if (busy) return;
    if (IS_MOBILE) fileRef.current?.click();
    else void pickImagePath().then((path) => path && run({ path }));
  };

  const onFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = '';
    if (file) run({ file });
  };

  const host = panel?.host.label;
  const recents = (panel?.recents ?? []).filter((r) => !hidden.has(r.url));

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-3 scrollbar-thin">
      <div className="flex flex-col gap-1.5">
        <button
          type="button"
          onClick={choose}
          disabled={busy || !panel}
          className="glass-tile flex min-h-[112px] w-full flex-col items-center justify-center gap-1.5 px-4 py-4 text-center transition-colors hover:bg-white/[0.03] disabled:cursor-default disabled:opacity-70"
        >
          {busy ? (
            <Loader2 size={22} className="animate-spin text-textSecondary" aria-hidden />
          ) : (
            <ImageUp size={22} className="text-textSecondary" aria-hidden />
          )}
          <span className="text-[13px] font-medium text-textPrimary">
            {busy ? `Uploading to ${host ?? 'the host'}…` : 'Choose an image'}
          </span>
          {/* Kept (only hidden) while busy, so the lines above do not move. */}
          <span className={`text-[11px] text-textSecondary ${busy ? 'invisible' : ''}`}>
            JPG, PNG, GIF or WebP{host ? `, to ${host}` : ''}
          </span>
        </button>
        {error && <p className="px-1 text-xs leading-relaxed text-error">{error}</p>}
      </div>

      {recents.length > 0 && (
        <div className="flex flex-col gap-2">
          <h3 className="px-1 text-[10px] font-bold uppercase tracking-wider text-textSecondary">Recent</h3>
          <div className={`grid gap-2 ${IS_MOBILE ? 'grid-cols-3' : 'grid-cols-4'}`}>
            {recents.map((r) => (
              <button
                key={r.url}
                type="button"
                onClick={() => onInsert(r.url)}
                aria-label="Insert this image's link"
                title="Insert this image's link"
                className="glass-tile aspect-square overflow-hidden rounded-lg transition-opacity hover:opacity-90"
              >
                <img
                  src={r.url}
                  alt=""
                  loading="lazy"
                  decoding="async"
                  referrerPolicy="no-referrer"
                  className="h-full w-full object-cover"
                  onError={() => setHidden((prev) => new Set(prev).add(r.url))}
                />
              </button>
            ))}
          </div>
        </div>
      )}

      <input ref={fileRef} type="file" accept="image/*" hidden onChange={onFile} />
    </div>
  );
}
