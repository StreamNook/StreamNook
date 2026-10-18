// Your profile, the first section of Edit profile: your two pictures (Twitch's
// and 7TV's, never blended into one), your Twitch banner, your display name
// (its capitalization) and your bio, each a row of one panel. Rust writes
// Twitch through the twitch.tv web session and 7TV through the 7TV session,
// and answers in words when either says no; this only picks, crops and shows.
// The iPhone's twin is You/TwitchProfileSection.swift.
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ChevronRight } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { MobileSheet } from '../ui/MobileSheet';
import { Button } from '../ui/Button';
import { Logger } from '../../utils/logger';
import { BrandMark } from './IdentityParts';
import { usePicksStore } from './picksStore';
import type { OwnIdentityView } from '../../types/profileView';

type ImageKind = 'picture' | 'banner' | 'sevenTV';

/** 7TV takes up to 7 MB. */
const SEVENTV_LIMIT = 7 * 1024 * 1024;
/** Twitch keeps 300 characters of bio (Rust checks it again before the write). */
const BIO_LIMIT = 300;

const errorText = (err: unknown): string => (err instanceof Error ? err.message : String(err ?? ''));

const blobToBase64 = (blob: Blob): Promise<string> =>
  new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(',')[1] ?? '');
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });

const isGif = async (file: Blob): Promise<boolean> => {
  const head = new Uint8Array(await file.slice(0, 4).arrayBuffer());
  return head.length === 4 && String.fromCharCode(...head) === 'GIF8';
};

/**
 * Cuts the pick to the shape it is shown in (square pictures at 600 px, the
 * banner 5:2 at 1200 by 480, all center-cropped) and sizes it down as a JPEG.
 * An animated GIF goes to 7TV untouched, so it keeps moving.
 */
async function prepareImage(file: File, kind: ImageKind): Promise<{ base64: string; preview: string } | null> {
  if (kind === 'sevenTV' && file.size <= SEVENTV_LIMIT && (await isGif(file))) {
    return { base64: await blobToBase64(file), preview: URL.createObjectURL(file) };
  }
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    return null;
  }
  const [w, h] = kind === 'banner' ? [1200, 480] : [600, 600];
  if (!bitmap.width || !bitmap.height) return null;
  const scale = Math.max(w / bitmap.width, h / bitmap.height);
  const dw = bitmap.width * scale;
  const dh = bitmap.height * scale;
  const canvas = document.createElement('canvas');
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  ctx.fillStyle = '#000';
  ctx.fillRect(0, 0, w, h);
  ctx.drawImage(bitmap, (w - dw) / 2, (h - dh) / 2, dw, dh);
  bitmap.close();
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/jpeg', 0.88));
  if (!blob) return null;
  return { base64: await blobToBase64(blob), preview: URL.createObjectURL(blob) };
}

/** A row's action: the phone's row button. */
const ChangePill: React.FC<{
  label: string;
  onClick: () => void;
  disabled?: boolean;
  ariaLabel?: string;
}> = ({ label, onClick, disabled, ariaLabel }) => (
  <Button size="row" onClick={onClick} disabled={disabled} aria-label={ariaLabel} className="whitespace-nowrap">
    {label}
  </Button>
);

/** As tall as the row button it stands in for, so the row keeps its height. */
const Spinner: React.FC = () => (
  <span className="h-[34px] w-11 shrink-0 flex items-center justify-center" aria-label="Uploading">
    <span className="w-5 h-5 rounded-full border-2 border-textSecondary/30 border-t-textSecondary animate-spin" />
  </span>
);

/** A 44 px round picture with its platform's mark on a disc at the corner, so
 *  each picture says whose it is without words. */
const PictureFrame: React.FC<{ brand: 'twitch' | 'seventv'; children: React.ReactNode }> = ({ brand, children }) => (
  <div className="relative shrink-0" aria-hidden="true">
    {/* glass-exempt: pure black, like every picture of ours, so a picture with
        transparent parts never shows the panel through it. */}
    <div className="w-11 h-11 rounded-full overflow-hidden ring-1 ring-white/10" style={{ background: '#000' }}>
      {children}
    </div>
    <span
      className="absolute -bottom-[3px] -right-1 rounded-full bg-tertiary flex items-center justify-center"
      style={{ width: 17, height: 17 }}
    >
      <BrandMark brand={brand} size={brand === 'seventv' ? 9 : 13} />
    </span>
  </div>
);

const RowText: React.FC<{ title: string; detail: string }> = ({ title, detail }) => (
  <div className="flex-1 min-w-0">
    <div className="text-[15px] font-semibold text-textPrimary truncate">{title}</div>
    <div className="text-[12.5px] text-textMuted truncate">{detail}</div>
  </div>
);

export interface ProfileEditSectionProps {
  userId: string;
  login: string;
  displayName: string;
  /** Your Twitch picture as Twitch serves it. */
  platformUrl?: string | null;
  identity: OwnIdentityView | null;
  sevenTvSignedIn: boolean;
  sevenTvUserId: string | null;
  /** A Twitch picture just picked, shown before Twitch serves it. */
  localPicture: string | null;
  onLocalPicture: (url: string | null) => void;
  /** The name as typed (null while untouched), shown in the preview too. */
  draftName: string | null;
  onDraftName: (name: string | null) => void;
  onConnectSevenTv: () => void;
}

export const ProfileEditSection: React.FC<ProfileEditSectionProps> = ({
  userId,
  login,
  displayName,
  platformUrl,
  identity,
  sevenTvSignedIn,
  sevenTvUserId,
  localPicture,
  onLocalPicture,
  draftName,
  onDraftName,
  onConnectSevenTv,
}) => {
  const addToast = useAppStore((s) => s.addToast);
  const bio = usePicksStore((s) => (s.userId === userId ? s.bio : null));
  const connecting = usePicksStore((s) => s.connecting);
  const fileInput = useRef<HTMLInputElement>(null);
  const pendingKind = useRef<ImageKind | null>(null);
  const [uploading, setUploading] = useState<ImageKind | null>(null);
  const [localBanner, setLocalBanner] = useState<string | null>(null);
  // A 7TV picture sent and waiting on 7TV's review.
  const [sevenTvPending, setSevenTvPending] = useState(false);
  const [savingName, setSavingName] = useState(false);
  const [editingBio, setEditingBio] = useState(false);

  // Object URLs for previews live as long as this section; free them after.
  const made = useRef<string[]>([]);
  useEffect(() => {
    const urls = made.current;
    return () => urls.forEach((u) => URL.revokeObjectURL(u));
  }, []);

  const toast = (message: string) => addToast(message, 'error');

  const pick = (kind: ImageKind) => {
    if (uploading) return;
    pendingKind.current = kind;
    fileInput.current?.click();
  };

  const refreshIdentity = () =>
    void usePicksStore.getState().loadIdentity({ userId, login, displayName });

  const onFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    const kind = pendingKind.current;
    e.target.value = '';
    pendingKind.current = null;
    if (!file || !kind) return;
    const prepared = await prepareImage(file, kind).catch(() => null);
    if (!prepared) {
      toast('Could not read that image.');
      return;
    }
    made.current.push(prepared.preview);
    setUploading(kind);
    try {
      if (kind === 'sevenTV') {
        await uploadSevenTv(prepared.base64);
        return;
      }
      if (kind === 'picture') onLocalPicture(prepared.preview);
      else setLocalBanner(prepared.preview);
      try {
        await invoke('upload_twitch_profile_image', { kind, dataBase64: prepared.base64 });
        // Twitch serves the new image a few seconds later; read it back after.
        await new Promise((r) => setTimeout(r, 4000));
        if (kind === 'picture') void useAppStore.getState().checkAuthStatus();
        refreshIdentity();
      } catch (err) {
        Logger.warn('[EditProfile] Twitch image upload failed:', err);
        if (kind === 'picture') onLocalPicture(null);
        else setLocalBanner(null);
        toast(errorText(err) || 'Twitch did not take that image. Try again in a moment.');
      }
    } finally {
      setUploading(null);
    }
  };

  const uploadSevenTv = async (base64: string) => {
    if (!sevenTvUserId) {
      toast('7TV did not identify your account.');
      return;
    }
    try {
      await invoke<string>('upload_seventv_profile_picture', { userId: sevenTvUserId, dataBase64: base64 });
      setSevenTvPending(true);
    } catch (err) {
      Logger.warn('[EditProfile] 7TV picture upload failed:', err);
      const text = errorText(err);
      if (text.includes('SESSION_EXPIRED')) {
        usePicksStore.getState().sevenTvExpired();
        toast('Your 7TV sign-in expired. Connect again, then pick the picture again.');
      } else {
        toast(text || '7TV did not take that picture.');
      }
    }
  };

  // The name as typed; Twitch lets you change only how your login is capitalized.
  const typed = draftName ?? displayName;
  const changed = typed.length > 0 && typed !== displayName;
  const sameName = typed.toLowerCase() === (login || displayName).toLowerCase();

  const saveName = async () => {
    if (savingName || !changed || !sameName) return;
    setSavingName(true);
    try {
      await invoke('update_twitch_display_name', { displayName: typed });
      onDraftName(null);
      await useAppStore.getState().checkAuthStatus();
    } catch (err) {
      Logger.warn('[EditProfile] display name write failed:', err);
      toast(errorText(err) || 'Could not change your name.');
    } finally {
      setSavingName(false);
    }
  };

  const bannerUrl = localBanner ?? identity?.banner_url ?? null;
  const sevenTvPicture = identity?.picture;

  return (
    <div className="glass-panel divide-y divide-borderSubtle">
      <input
        ref={fileInput}
        type="file"
        accept="image/*"
        hidden
        onChange={(e) => void onFile(e)}
      />

      {/* Twitch picture */}
      <div className="flex items-center gap-3 px-3.5 py-2.5">
        <PictureFrame brand="twitch">
          {(localPicture ?? platformUrl) && (
            <img src={localPicture ?? platformUrl ?? ''} alt="" draggable={false} className="w-full h-full object-cover" />
          )}
        </PictureFrame>
        <RowText title="Twitch picture" detail="Shown on Twitch" />
        {uploading === 'picture' ? (
          <Spinner />
        ) : (
          <ChangePill
            label="Change"
            onClick={() => pick('picture')}
            disabled={!!uploading}
            ariaLabel="Change your Twitch picture"
          />
        )}
      </div>

      {/* 7TV picture */}
      <div className="flex items-center gap-3 px-3.5 py-2.5">
        <PictureFrame brand="seventv">
          {sevenTvPicture ? (
            <img
              src={sevenTvPicture.animated ?? sevenTvPicture.still}
              alt=""
              draggable={false}
              className="w-full h-full object-cover"
            />
          ) : (
            <span className="w-full h-full flex items-center justify-center">
              <BrandMark brand="seventv" size={12} color="var(--color-text-muted)" />
            </span>
          )}
        </PictureFrame>
        <RowText title="7TV picture" detail={sevenTvPending ? 'Processing on 7TV' : 'Shown in chat'} />
        {uploading === 'sevenTV' ? (
          <Spinner />
        ) : sevenTvSignedIn ? (
          <ChangePill
            label="Change"
            onClick={() => pick('sevenTV')}
            disabled={!!uploading}
            ariaLabel="Change your 7TV picture"
          />
        ) : (
          // Signed out of 7TV: connect first, as the paint tiles do.
          <ChangePill
            label={connecting ? 'Connecting' : 'Connect'}
            onClick={onConnectSevenTv}
            disabled={connecting}
            ariaLabel="Connect 7TV to change your 7TV picture"
          />
        )}
      </div>

      {/* Banner */}
      <div className="px-3.5 py-3">
        <div className="flex items-center gap-3">
          <RowText title="Banner" detail="Across your channel page" />
          {uploading === 'banner' ? (
            <Spinner />
          ) : (
            <ChangePill
              label="Change"
              onClick={() => pick('banner')}
              disabled={!!uploading}
              ariaLabel="Change your banner"
            />
          )}
        </div>
        {bannerUrl && (
          <img
            src={bannerUrl}
            alt=""
            draggable={false}
            aria-hidden="true"
            className="mt-2.5 w-full h-16 rounded-[10px] object-cover"
          />
        )}
      </div>

      {/* Display name */}
      <div className="flex items-center gap-3 px-3.5 py-2.5">
        <label className="flex-1 min-w-0">
          {/* The warning takes the label's place rather than a line of its own,
              so nothing below moves while you type. */}
          <span className={`block text-[12.5px] truncate ${changed && !sameName ? 'text-warning' : 'text-textMuted'}`}>
            {changed && !sameName ? 'Only the capitalization of your name can change' : 'Display name'}
          </span>
          <input
            value={typed}
            onChange={(e) => onDraftName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void saveName();
            }}
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            enterKeyHint="done"
            maxLength={25}
            className="block w-full bg-transparent text-[15px] font-semibold text-textPrimary outline-none"
          />
        </label>
        {changed && (
          <Button size="row" kind="primary" onClick={() => void saveName()} disabled={!sameName || savingName}>
            {savingName ? 'Saving' : 'Save'}
          </Button>
        )}
      </div>

      {/* Bio */}
      {bio !== null && (
        <button
          onClick={() => setEditingBio(true)}
          className="w-full flex items-start gap-3 px-3.5 py-2.5 text-left active:bg-surface-active"
        >
          <span className="flex-1 min-w-0">
            <span className="block text-[12.5px] text-textMuted">Bio</span>
            <span
              className={`block text-[14px] line-clamp-3 break-words ${bio ? 'text-textPrimary' : 'text-textSecondary'}`}
            >
              {bio || 'Add a few words about you'}
            </span>
          </span>
          <ChevronRight size={16} className="text-textMuted shrink-0 mt-3.5" />
        </button>
      )}

      <BioSheet open={editingBio} initial={bio ?? ''} onClose={() => setEditingBio(false)} />
    </div>
  );
};

/** Writes your Twitch bio: the count shows as you type and Save stays off past
 *  Twitch's limit. */
const BioSheet: React.FC<{ open: boolean; initial: string; onClose: () => void }> = ({ open, initial, onClose }) => {
  const addToast = useAppStore((s) => s.addToast);
  const [text, setText] = useState(initial);
  const [saving, setSaving] = useState(false);
  // Opening starts from the bio as it is now (adjust-state-during-render, so the
  // first frame of the sheet already holds it).
  const [openedWith, setOpenedWith] = useState<string | null>(null);
  if (open && openedWith === null) {
    setOpenedWith(initial);
    setText(initial);
  } else if (!open && openedWith !== null) {
    setOpenedWith(null);
  }

  const count = [...text].length;
  const over = count > BIO_LIMIT;

  const save = async () => {
    if (saving || over || text === initial) return;
    setSaving(true);
    try {
      const stored = await invoke<string>('update_twitch_bio', { description: text });
      usePicksStore.getState().setBio(stored);
      onClose();
    } catch (err) {
      Logger.warn('[EditProfile] bio write failed:', err);
      addToast(errorText(err) || 'Could not save your bio.', 'error');
    } finally {
      setSaving(false);
    }
  };

  return (
    <MobileSheet open={open} onClose={onClose} title="Bio" maxHeightFraction={0.8}>
      <div className="flex flex-col gap-2 pt-1">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={6}
          className="glass-panel w-full min-h-[140px] p-2.5 text-[15px] text-textPrimary outline-none resize-none"
          placeholder="A few words about you"
        />
        <div className={`self-end text-[12px] tabular-nums ${over ? 'text-error' : 'text-textMuted'}`}>
          {count}/{BIO_LIMIT}
        </div>
        <div className="flex gap-3 pt-1">
          <Button onClick={onClose} className="flex-1">
            Cancel
          </Button>
          <Button kind="primary" onClick={() => void save()} disabled={saving || over || text === initial} className="flex-1">
            {saving ? 'Saving' : 'Save'}
          </Button>
        </div>
      </div>
    </MobileSheet>
  );
};
