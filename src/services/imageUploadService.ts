// The chat image uploader's IPC, for the emote menu's Images section, paste
// and drops. Rust (services/image_upload.rs) owns the host, the checks and the
// post; this file only gets the picture there and runs the one-time host
// question. TypeScript because the pickers, the dialog and FileReader are
// webview APIs, and kept out of components so the React Compiler can compile
// them (it refuses components with try/finally).

import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { ask, open } from '@tauri-apps/plugin-dialog';
import { IS_MOBILE } from '../utils/platform';
import { useAppStore } from '../stores/AppStore';
import type { ImageUploadPanel, UploadedImage } from '../types';

export const IMAGE_EXTENSIONS = ['jpg', 'jpeg', 'png', 'gif', 'webp', 'avif', 'bmp'];
/** Also accepted from a drop, so Rust can say why HEIC does not work. */
const DROP_EXTENSIONS = [...IMAGE_EXTENSIONS, 'heic', 'heif'];

export type UploadSource = { file: Blob } | { path: string };
export type UploadOutcome =
  | { status: 'done'; image: UploadedImage }
  | { status: 'cancelled' }
  | { status: 'failed'; error: string };

export function imageUploadPanel(): Promise<ImageUploadPanel> {
  return invoke<ImageUploadPanel>('image_upload_panel');
}

/** The Images tab setting, which also governs drops. */
export function imagesTabOn(): boolean {
  return useAppStore.getState().settings.chat_input?.image_uploader?.show_in_menu !== false;
}

export function isDroppableImagePath(path: string): boolean {
  return DROP_EXTENSIONS.includes(path.split('.').pop()?.toLowerCase() ?? '');
}

/** The desktop picker. Android uses a file input (ImageUploadSection). */
export async function pickImagePath(): Promise<string | null> {
  if (IS_MOBILE) return null;
  const picked = await open({
    multiple: false,
    directory: false,
    filters: [{ name: 'Images', extensions: IMAGE_EXTENSIONS }],
  });
  return typeof picked === 'string' ? picked : null;
}

// One drop can reach a chat twice where a window both runs Tauri's native
// drop handler and lets the HTML5 drop through; the second copy is ignored.
let lastDrop = { name: '', at: 0 };
export function firstDropOf(name: string): boolean {
  const base = name.split(/[\\/]/).pop() ?? name;
  const now = Date.now();
  if (base === lastDrop.name && now - lastDrop.at < 1500) return false;
  lastDrop = { name: base, at: now };
  return true;
}

/** Desktop sends a File's bytes raw. Android's IPC is postMessage only, where
 *  raw bytes would become a JSON array of numbers, so the phone sends base64. */
function fileBody(file: Blob): Promise<Uint8Array | { dataBase64: string }> {
  if (!IS_MOBILE) return file.arrayBuffer().then((b) => new Uint8Array(b));
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      resolve({ dataBase64: url.slice(url.indexOf(',') + 1) });
    };
    reader.onerror = () => reject('That file could not be opened.');
    reader.readAsDataURL(file);
  });
}

async function confirmHost(panel: ImageUploadPanel): Promise<boolean> {
  if (!panel.needs_consent) return true;
  const yes = await ask(`Images you upload go to ${panel.host.label}, and anyone with the link can see them.`, {
    title: 'Send images to this host?',
    kind: 'info',
    okLabel: 'Upload',
    cancelLabel: 'Cancel',
  });
  if (yes) await invoke('accept_image_upload_host');
  return yes;
}

/** Asks about the host once, uploads, and asks again (once) if the host
 *  changed while this was open. `onStart` runs just before the upload. */
export async function uploadWithConsent(
  source: UploadSource,
  onStart?: (hostLabel: string) => void,
): Promise<UploadOutcome> {
  try {
    for (let attempt = 0; attempt < 2; attempt++) {
      const panel = await imageUploadPanel();
      if (!(await confirmHost(panel))) return { status: 'cancelled' };
      onStart?.(panel.host.label);
      const body = 'path' in source ? { path: source.path } : await fileBody(source.file);
      try {
        return { status: 'done', image: await invoke<UploadedImage>('upload_chat_image', body) };
      } catch (err) {
        if (err !== 'consent-required') throw err;
      }
    }
    return { status: 'failed', error: 'Image upload failed.' };
  } catch (err) {
    return { status: 'failed', error: typeof err === 'string' ? err : 'Image upload failed.' };
  }
}

export type ItemOutcome = { status: 'done'; image: UploadedImage } | { status: 'failed'; error: string };

const failed = (err: unknown, fallback: string) => ({
  status: 'failed' as const,
  error: typeof err === 'string' ? err : fallback,
});

/** Sends a kept copy again for a fresh link (it replaces the old entry). */
export function reuploadImage(url: string): Promise<ItemOutcome> {
  return invoke<UploadedImage>('reupload_image', { url }).then(
    (image) => ({ status: 'done' as const, image }),
    (err) => failed(err, 'Sending it again failed.'),
  );
}

/** Removes an upload from the list, deleting it on the host where it can. */
export function deleteUpload(url: string): Promise<{ status: 'done' } | { status: 'failed'; error: string }> {
  return invoke('delete_image_upload', { url }).then(
    () => ({ status: 'done' as const }),
    (err) => failed(err, 'Deleting it failed.'),
  );
}

/** Where the next upload goes: the same setting as Settings > Image Uploads. */
export function setUploadHost(id: string): Promise<void> {
  const { settings, updateSettings } = useAppStore.getState();
  const up = settings.chat_input?.image_uploader ?? {};
  return updateSettings({ ...settings, chat_input: { ...settings.chat_input, image_uploader: { ...up, preset: id } } });
}

/** How the Images tab shows uploads (a setting, so it sticks). */
export function setUploadView(view: 'grid' | 'list'): Promise<void> {
  const { settings, updateSettings } = useAppStore.getState();
  const up = settings.chat_input?.image_uploader ?? {};
  return updateSettings({ ...settings, chat_input: { ...settings.chat_input, image_uploader: { ...up, view } } });
}

/** Copies an upload's link. */
export function copyLink(url: string): Promise<boolean> {
  return navigator.clipboard.writeText(url).then(
    () => true,
    () => false,
  );
}

/** A file the app keeps (a small picture, a copy) as an image source. */
export function localSrc(path: string): string {
  return convertFileSrc(path);
}

/** Shows the folder the uploads are kept in (desktop). */
export function openUploadsFolder(): Promise<void> {
  return invoke<void>('open_image_uploads_folder').catch(() => {});
}

/** Paste and drop: upload, then put the link where the caret is. */
export async function uploadIntoBox(
  source: UploadSource,
  insert: { current: (text: string) => void },
): Promise<void> {
  const { addToast } = useAppStore.getState();
  const out = await uploadWithConsent(source, (host) => addToast(`Uploading image to ${host}…`, 'info'));
  if (out.status === 'done') insert.current(out.image.url);
  else if (out.status === 'failed') addToast(out.error, 'error');
}
