// Edit profile: how you look, pinned at the top and redrawn with every pick
// (your Atmosphere as the band, your picture, your painted name and badges, and
// a line of your chat as everyone sees it), then every choice laid open: your
// Twitch pictures, banner, name and bio, your Atmosphere, your StreamNook
// badge, your 7TV paint and badge, your Twitch badge and your name color.
// Pushed from the You tab's identity card. The iPhone's twin is
// You/CosmeticsScreen.swift.
//
// What you own, what you wear and which Atmospheres you may pick come from
// Rust in two answers (`cosmetics_picker_state`, `external_cosmetics_state`),
// held in the profile store the You card reads, so a pick here shows there at
// once. Every pick flips at once and is put back if the write is refused.
//
// The StreamNook writes are the desktop editor's (setActiveCosmetic,
// setProfileTheme, setChatAtmosphere). 7TV paints and badges go through
// set_seventv_paint / set_seventv_badge; what you own is public, so it lists
// whether or not you have connected 7TV, and the first tap on a tile is where
// connecting happens. Twitch's global badge goes through the GQL mutation the
// desktop editor uses, which needs the Drops sign-in; without it the earned
// badges still list, read-only. The name color is one of Twitch's named colors
// through Helix (`update_user_chat_color`).
import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, Lock } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { setOwnThemeHiddenInChat } from '../../stores/chatUserStore';
import { useMobileNavStore } from '../navStore';
import { DrillInScreen } from '../ui/DrillInScreen';
import { Button } from '../ui/Button';
import { FallbackImage } from '../../components/FallbackImage';
import { clearUserCache as clear7TVCache } from '../../services/seventvService';
import { applyLocalCosmeticSelection } from '../../services/cosmeticsCache';
import { setActiveCosmetic, setProfileTheme, setChatAtmosphere } from '../../services/supabaseService';
import { getAtmosphere } from '../../services/atmospheres';
import { CosmeticArt } from '../../components/CosmeticArt';
import { Logger } from '../../utils/logger';
import type {
  ChatIdentityBadge,
  ExternalPickerView,
  NamedChatColor,
  StreamNookPickerView,
} from '../../types/profileView';
import { usePicksStore, useOwnPicks } from '../profile/picksStore';
import {
  AtmosphereGlow,
  BrandTile,
  CutoutAvatar,
  PaintedName,
  ProfilePreviewRow,
  SectionHeader,
  WornBadge,
  WornMarks,
} from '../profile/IdentityParts';
import {
  chatAtmosphereId,
  glowAtmosphere,
  nameColorOr,
  SEVENTV_BLUE,
  SEVENTV_INK,
  wornPreviewBadges,
} from '../profile/identity';
import { ProfileEditSection } from '../profile/ProfileEditSection';

/** Paints and badges past this many wait behind "Show all". */
const SHORT = 8;

/** The first eight, with the one you wear swapped into the last place when it
 *  sits further down, so what you have on is always in view. */
function shortList<T>(items: T[], worn: (item: T) => boolean): T[] {
  const shown = items.slice(0, SHORT);
  if (!shown.some(worn)) {
    const wearing = items.find(worn);
    if (wearing && shown.length) shown[shown.length - 1] = wearing;
  }
  return shown;
}

const errorText = (err: unknown): string => (err instanceof Error ? err.message : String(err ?? ''));

/** A section's header with the spacing every section here shares. */
const Header: React.FC<React.ComponentProps<typeof SectionHeader>> = (props) => (
  <SectionHeader {...props} className="mt-6 mb-3" />
);

/** A badge tile: the art at 36 px on a tile, the worn one ringed and checked.
 *  The name is the section header's detail and the tile's label. A StreamNook
 *  badge passes `cosmetic`, whose art comes from Rust's disk copy and never
 *  draws a broken image. */
const BadgeTile: React.FC<{
  url?: string | null;
  fallbacks?: string[];
  cosmetic?: React.ComponentProps<typeof CosmeticArt>['cosmetic'];
  worn: boolean;
  label: string;
  onClick: () => void;
}> = ({ url, fallbacks, cosmetic, worn, label, onClick }) => (
  <button
    onClick={onClick}
    aria-label={label}
    aria-pressed={worn}
    className="relative w-full aspect-square rounded-xl bg-tertiary border border-borderSubtle flex items-center justify-center active:scale-90 transition-transform"
  >
    {cosmetic ? (
      <CosmeticArt cosmetic={cosmetic} className="w-9 h-9 object-contain" />
    ) : url ? (
      <FallbackImage src={url} fallbackUrls={fallbacks} alt="" draggable={false} className="w-9 h-9 object-contain" />
    ) : (
      <span className="w-9 h-9 rounded-md bg-surface" />
    )}
    {worn && <WornMarks radius={12} />}
  </button>
);

/** Five across, the first eight and then "Show all". */
function TileGrid<T>({
  items,
  open,
  worn,
  children,
}: {
  items: T[];
  open: boolean;
  worn: (item: T) => boolean;
  children: (item: T) => React.ReactNode;
}) {
  const shown = open || items.length <= SHORT + 2 ? items : shortList(items, worn);
  return <div className="grid grid-cols-5 gap-3">{shown.map((item) => children(item))}</div>;
}

const ShowAll: React.FC<{ count: number; open: boolean; onToggle: () => void }> = ({ count, open, onToggle }) =>
  count > SHORT + 2 ? (
    <button
      onClick={onToggle}
      className="w-full min-h-[44px] mt-0.5 -mb-3 text-[14px] font-semibold text-accent active:opacity-70"
    >
      {open ? 'Show fewer' : `Show all ${count}`}
    </button>
  ) : null;

export const CosmeticsScreen: React.FC = () => {
  const open = useMobileNavStore((s) => s.cosmeticsOpen);
  const setCosmeticsOpen = useMobileNavStore((s) => s.setCosmeticsOpen);
  const currentUser = useAppStore((s) => s.currentUser);
  const addToast = useAppStore((s) => s.addToast);

  const userId = currentUser?.user_id ?? '';
  const login = currentUser?.login || currentUser?.username || '';
  const displayName = currentUser?.display_name || login;

  // Every open re-reads everything (each half lands on its own), and a
  // registry change while open re-reads the StreamNook half.
  useOwnPicks(userId ? { userId, login, displayName } : null, open);
  const identity = usePicksStore((s) => (s.userId === userId ? s.identity : null));
  const sn = usePicksStore((s) => (s.userId === userId ? s.sn : null));
  const ext = usePicksStore((s) => (s.userId === userId ? s.ext : null));
  const sevenTv = usePicksStore((s) => (s.userId === userId ? s.sevenTv : null));
  const connecting = usePicksStore((s) => s.connecting);
  const { patchSn, patchExt, setSn, setExt } = usePicksStore.getState();

  // A picture just picked and the name as typed, shown in the pinned preview
  // before Twitch has them. Both drop when the screen closes.
  const [localPicture, setLocalPicture] = useState<string | null>(null);
  const [draftName, setDraftName] = useState<string | null>(null);
  const [openSections, setOpenSections] = useState<Set<string>>(new Set());
  const [wasOpen, setWasOpen] = useState(open);
  if (wasOpen !== open) {
    setWasOpen(open);
    if (!open) {
      setLocalPicture(null);
      setDraftName(null);
    }
  }

  const isOpen = (key: string) => openSections.has(key);
  const toggleSection = (key: string) =>
    setOpenSections((prev) => {
      const next = new Set(prev);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  const sevenTvSignedIn = sevenTv?.signedIn ?? ext?.seventv_signed_in ?? false;
  const sevenTvUserId = sevenTv?.userId ?? ext?.seventv_user_id ?? null;
  const wornPaint = ext ? (ext.paints.find((p) => p.worn) ?? null) : null;
  // Once the 7TV half has answered it is the truth, a paint taken off included.
  const paintDef = ext ? (wornPaint?.def ?? null) : (identity?.paint_def ?? null);
  const shownName = draftName || displayName;

  // StreamNook

  /** Wears a badge; tapping the one worn takes it off (back to the plain mark). */
  const equipBadge = async (badge: StreamNookPickerView['badges'][number]) => {
    if (!sn) return;
    const before = sn;
    const next = badge.worn ? null : badge.slug;
    patchSn((s) => ({ ...s, badges: s.badges.map((b) => ({ ...b, worn: b.slug === next })) }));
    const res = await setActiveCosmetic(userId, next);
    if (!res.ok) {
      setSn(before);
      addToast(res.error || 'Could not equip that badge.', 'error');
    }
  };

  const equipTheme = async (atm: StreamNookPickerView['atmospheres'][number]) => {
    if (!sn || !atm.allowed) return;
    const before = sn;
    patchSn((s) => ({
      ...s,
      theme: atm.id,
      atmospheres: s.atmospheres.map((a) => ({ ...a, worn: a.id === atm.id })),
      show_chat_toggle: !!getAtmosphere(atm.id),
    }));
    try {
      await setProfileTheme(userId, atm.id);
    } catch (err) {
      Logger.error('[EditProfile] theme write failed:', err);
      setSn(before);
      addToast('Could not apply that Atmosphere.', 'error');
    }
  };

  const toggleChatAtmosphere = async () => {
    if (!sn || !sn.show_chat_toggle) return;
    const next = !sn.chat_atmosphere;
    patchSn((s) => ({ ...s, chat_atmosphere: next }));
    // Repaint our own rows now rather than waiting for the realtime echo.
    setOwnThemeHiddenInChat(userId, !next);
    if (!(await setChatAtmosphere(userId, next))) {
      patchSn((s) => ({ ...s, chat_atmosphere: !next }));
      setOwnThemeHiddenInChat(userId, next);
      addToast('Could not save that. Try again in a moment.', 'error');
    }
  };

  // 7TV

  /** Runs `apply` with the 7TV account id, connecting first when needed: the
   *  first tap on a tile is the natural place for it. */
  const withSevenTv = async (apply: (sevenTvId: string) => Promise<void>) => {
    let id = sevenTvUserId;
    if (!sevenTvSignedIn) {
      id = await usePicksStore.getState().connectSevenTv(userId);
      if (!id) return;
    }
    if (!id) {
      addToast('7TV did not identify your account.', 'error');
      return;
    }
    await apply(id);
  };

  const connectSevenTv = () => void withSevenTv(async () => {});

  /**
   * A refused write usually means the session lapsed, and the only way to find
   * out is to try. Dropping the connected flag sends the next tap back through
   * Connect instead of failing the same way forever.
   */
  const sevenTvWriteFailed = (err: unknown, what: string) => {
    Logger.error(`[EditProfile] 7TV ${what} write failed:`, err);
    if (errorText(err).includes('SESSION_EXPIRED')) {
      usePicksStore.getState().sevenTvExpired();
      addToast('Your 7TV sign-in expired. Tap again to connect.', 'error');
    } else {
      addToast(`Could not equip that ${what}.`, 'error');
    }
  };

  // Both 7TV writes flip the pick locally rather than re-reading it: 7TV's read
  // lags its own write by a few seconds, so a re-read would show the PREVIOUS
  // pick. Clearing the lower-level 7TV cache keeps a later natural read fresh.
  const equipPaint = (paint: ExternalPickerView['paints'][number]) =>
    withSevenTv(async (sevenTvId) => {
      const before = usePicksStore.getState().ext;
      if (!before) return;
      const next = before.paints.find((p) => p.worn)?.id === paint.id ? null : paint.id;
      patchExt((e) => ({ ...e, paints: e.paints.map((p) => ({ ...p, worn: p.id === next })) }));
      try {
        const res = await invoke<{ success: boolean }>('set_seventv_paint', { userId: sevenTvId, paintId: next });
        if (!res.success) throw new Error('7TV refused the paint change');
        clear7TVCache();
        applyLocalCosmeticSelection(userId, { paintId: next });
      } catch (err) {
        setExt(before);
        sevenTvWriteFailed(err, 'paint');
      }
    });

  const equipSevenTvBadge = (entry: ExternalPickerView['seventv_badges'][number]) =>
    withSevenTv(async (sevenTvId) => {
      const before = usePicksStore.getState().ext;
      if (!before) return;
      const id = entry.badge.id;
      const next = before.seventv_badges.find((b) => b.worn)?.badge.id === id ? null : id;
      patchExt((e) => ({
        ...e,
        seventv_badges: e.seventv_badges.map((b) => ({ ...b, worn: b.badge.id === next })),
      }));
      try {
        const res = await invoke<{ success: boolean }>('set_seventv_badge', { userId: sevenTvId, badgeId: next });
        if (!res.success) throw new Error('7TV refused the badge change');
        clear7TVCache();
        applyLocalCosmeticSelection(userId, { badgeId: next });
      } catch (err) {
        setExt(before);
        sevenTvWriteFailed(err, 'badge');
      }
    });

  // Twitch

  /** Wears a Twitch global badge. There is no taking one off: Twitch's editor
   *  only swaps which is worn, so tapping the worn one does nothing. */
  const equipTwitchBadge = async (badge: ChatIdentityBadge) => {
    if (badge.is_selected || !ext) return;
    const before = ext;
    patchExt((e) => ({
      ...e,
      twitch_selectable: e.twitch_selectable.map((b) => ({ ...b, is_selected: b.id === badge.id })),
    }));
    try {
      await invoke('update_chat_identity_gql', { badgeId: badge.id, badgeVersion: badge.version });
    } catch (err) {
      Logger.error('[EditProfile] Twitch badge write failed:', err);
      setExt(before);
      addToast(
        errorText(err).includes('NEEDS_DROPS_AUTH')
          ? 'Sign in to Drops to change your Twitch badge.'
          : 'Could not equip that badge.',
        'error',
      );
    }
  };

  /** Sets your Twitch name color to one of Twitch's named colors. */
  const setChatColor = async (named: NamedChatColor) => {
    if (!ext || ext.chat_color?.toLowerCase() === named.hex.toLowerCase()) return;
    const before = ext;
    patchExt((e) => ({ ...e, chat_color: named.hex }));
    try {
      await invoke('update_user_chat_color', { targetUserId: userId, color: named.value });
    } catch (err) {
      Logger.error('[EditProfile] name color write failed:', err);
      setExt(before);
      addToast('Could not change your name color.', 'error');
    }
  };

  const snWornBadge = sn?.badges.find((b) => b.worn);
  const noAtmosphereWorn = !!sn && !sn.atmospheres.some((a) => a.worn);
  const color = nameColorOr(ext?.chat_color);
  const chatHex = ext?.chat_color?.toLowerCase();
  const currentColor = ext?.named_colors.find((c) => c.hex.toLowerCase() === chatHex);
  const previewBadges = wornPreviewBadges(sn, ext);

  return (
    <DrillInScreen
      open={open}
      layerKey="cosmetics"
      className="absolute inset-0 z-50 bg-background flex flex-col"
      style={{ paddingTop: 'var(--sn-safe-t, 0px)' }}
    >
      <div className="flex items-center gap-1 px-2 py-2 shrink-0">
        <button
          onClick={() => setCosmeticsOpen(false)}
          className="sn-touch flex items-center justify-center text-textSecondary"
          aria-label="Back"
        >
          <ArrowLeft size={22} />
        </button>
        <h1 className="text-[17px] font-bold text-textPrimary">Edit profile</h1>
      </div>

      {/* You as others see you, held above the choices while they scroll.
          Solid at any Glassiness: nothing passes through it. */}
      {userId && (
        <div className="relative shrink-0 px-4 pt-1 pb-3">
          {/* glass-exempt: the page's own background fading out just under the
              card, so the choices slide softly under it instead of being cut
              at its edge. Over the list, never in the way of a tap. */}
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 top-full z-10 h-6"
            style={{ background: 'linear-gradient(var(--color-background), transparent)' }}
          />
          <div
            className="relative overflow-hidden rounded-[18px] bg-secondary pb-3"
            style={{ boxShadow: '0 8px 12px rgba(0,0,0,0.55)' }}
          >
            {/* Your Atmosphere glowing behind you, as on the You card. */}
            <AtmosphereGlow atm={glowAtmosphere(sn ? sn.theme : identity?.atmosphere?.id)} tint={color} height={120} />
            <div className="relative flex items-center gap-2.5 px-3 pt-3">
              <CutoutAvatar
                platformUrl={currentUser?.profile_image_url}
                picture={identity?.picture}
                size={56}
                localUrl={localPicture}
              />
              <div className="flex items-center gap-1.5 min-w-0">
                <PaintedName
                  text={shownName}
                  paint={paintDef}
                  color={color}
                  className="text-[19px] font-bold whitespace-nowrap overflow-hidden text-ellipsis min-w-0"
                />
                {previewBadges.map((url, i) => (
                  <WornBadge key={`${url}-${i}`} url={url} />
                ))}
              </div>
            </div>
            {(sn || ext) && (
              <div className="relative px-3 pt-3">
                <ProfilePreviewRow
                  name={shownName}
                  paint={paintDef}
                  colorHex={ext?.chat_color}
                  badges={previewBadges}
                  atmosphereId={chatAtmosphereId(sn)}
                />
              </div>
            )}
          </div>
        </div>
      )}

      <div
        className="flex-1 min-h-0 overflow-y-auto px-4"
        style={{ paddingBottom: 'calc(var(--sn-safe-b, 0px) + 32px)' }}
      >
        {userId && (
          <>
            <SectionHeader title="Profile" brand="twitch" className="mt-2 mb-3" />
            <ProfileEditSection
              userId={userId}
              login={login}
              displayName={displayName}
              platformUrl={currentUser?.profile_image_url}
              identity={identity}
              sevenTvSignedIn={sevenTvSignedIn}
              sevenTvUserId={sevenTvUserId}
              localPicture={localPicture}
              onLocalPicture={setLocalPicture}
              draftName={draftName}
              onDraftName={setDraftName}
              onConnectSevenTv={connectSevenTv}
            />
          </>
        )}

        {/* Atmosphere */}
        <Header title="Atmosphere" brand="streamnook" />
        {sn && (
          <>
            <div className="flex gap-3 overflow-x-auto scrollbar-none mask-edges -mx-4 px-4 py-1.5">
              {sn.atmospheres.map((atm) => {
                const chosen = atm.worn || (atm.id === 'tier' && noAtmosphereWorn);
                const swatch = atm.id === 'tier' ? null : getAtmosphere(atm.id)?.swatch;
                return (
                  <button
                    key={atm.id}
                    onClick={() => void equipTheme(atm)}
                    disabled={!atm.allowed}
                    aria-pressed={chosen}
                    className="shrink-0 w-[116px] text-left active:scale-95 transition-transform disabled:opacity-45"
                  >
                    <span
                      className="relative block w-[116px] h-[66px] rounded-[10px] bg-surface"
                      style={swatch ? { background: swatch } : undefined}
                    >
                      {chosen && <WornMarks radius={10} />}
                    </span>
                    <span className="mt-1.5 flex items-center gap-1 min-w-0">
                      {!atm.allowed && <Lock size={10} weight="bold" className="text-textMuted shrink-0" />}
                      <span
                        className={`text-[12.5px] truncate ${
                          chosen ? 'font-semibold text-textPrimary' : 'font-medium text-textSecondary'
                        }`}
                      >
                        {atm.id === 'tier' ? 'None' : atm.name}
                      </span>
                    </span>
                  </button>
                );
              })}
            </div>
            {/* Always here, so picking None never moves what is below; off and
                dimmed while nothing can show in chat. */}
            <div
              className={`glass-panel mt-3 flex items-center justify-between gap-3 px-3.5 min-h-[50px] ${
                sn.show_chat_toggle ? '' : 'opacity-40'
              }`}
            >
              <span className="text-[15px] font-medium text-textPrimary">Show in my chat messages</span>
              <button
                onClick={() => void toggleChatAtmosphere()}
                disabled={!sn.show_chat_toggle}
                role="switch"
                aria-checked={sn.show_chat_toggle && sn.chat_atmosphere}
                aria-label="Show your Atmosphere in your chat messages"
                className={`shrink-0 w-11 h-6 rounded-full relative transition-colors ${
                  sn.show_chat_toggle && sn.chat_atmosphere ? 'bg-accent' : 'bg-surface'
                }`}
              >
                <span
                  className="absolute top-0.5 w-5 h-5 rounded-full bg-white transition-[left] duration-200 ease-out"
                  style={{ left: sn.show_chat_toggle && sn.chat_atmosphere ? 22 : 2 }}
                />
              </button>
            </div>
          </>
        )}

        {/* StreamNook badge */}
        <Header title="Badge" brand="streamnook" detail={snWornBadge?.name} />
        {sn &&
          (sn.badges.length === 0 ? (
            <div className="glass-panel p-3.5 text-[14px] text-textSecondary">
              No badges yet. Earned and event badges land here.
            </div>
          ) : (
            <>
              <TileGrid items={sn.badges} open={isOpen('snBadges')} worn={(b) => b.worn}>
                {(badge) => (
                  <BadgeTile
                    key={badge.slug}
                    cosmetic={{ slug: badge.slug, asset_path: badge.image_url ?? null }}
                    worn={badge.worn}
                    label={badge.name}
                    onClick={() => void equipBadge(badge)}
                  />
                )}
              </TileGrid>
              <ShowAll count={sn.badges.length} open={isOpen('snBadges')} onToggle={() => toggleSection('snBadges')} />
            </>
          ))}

        {/* 7TV */}
        {ext && (ext.paints.length > 0 || ext.seventv_badges.length > 0 || !sevenTvSignedIn) && (
          <>
            <Header title="Paint" brand="seventv" detail={wornPaint?.name} />
            {!sevenTvSignedIn && (
              <div className={`glass-panel p-3 flex items-center gap-3 ${ext.paints.length ? 'mb-2.5' : ''}`}>
                <BrandTile brand="seventv" size={40} color={SEVENTV_BLUE} />
                <span className="flex-1 min-w-0 text-[14px] text-textSecondary">
                  {ext.paints.length > 0
                    ? `Connect 7TV to switch paints. You own ${ext.paints.length}.`
                    : 'Connect 7TV to wear your paints and badges.'}
                </span>
                <Button
                  size="row"
                  kind="brand"
                  onClick={connectSevenTv}
                  disabled={connecting}
                  className="whitespace-nowrap"
                  style={{ backgroundColor: SEVENTV_BLUE, color: SEVENTV_INK }}
                >
                  {connecting ? 'Connecting' : 'Connect'}
                </Button>
              </div>
            )}
            {ext.paints.length > 0 && (
              <>
                <div className="grid grid-cols-2 gap-2">
                  {(isOpen('paints') || ext.paints.length <= SHORT + 2
                    ? ext.paints
                    : shortList(ext.paints, (p) => p.worn)
                  ).map((paint) => (
                    <button
                      key={paint.id}
                      onClick={() => void equipPaint(paint)}
                      aria-pressed={paint.worn}
                      className="relative glass-panel h-[46px] px-3 flex items-center text-left active:scale-95 transition-transform"
                    >
                      {/* The paint IS the preview: its own name drawn in it. */}
                      <PaintedName
                        text={paint.name}
                        paint={paint.def}
                        color="var(--color-text-primary)"
                        className="text-[15px] font-bold whitespace-nowrap overflow-hidden text-ellipsis min-w-0"
                      />
                      {paint.worn && <WornMarks radius={12} />}
                    </button>
                  ))}
                </div>
                <ShowAll count={ext.paints.length} open={isOpen('paints')} onToggle={() => toggleSection('paints')} />
              </>
            )}
            {ext.seventv_badges.length > 0 && (
              <>
                <Header
                  title="Badge"
                  brand="seventv"
                  detail={ext.seventv_badges.find((b) => b.worn)?.badge.title || undefined}
                />
                <TileGrid items={ext.seventv_badges} open={isOpen('seventvBadges')} worn={(b) => b.worn}>
                  {(entry) => (
                    <BadgeTile
                      key={entry.badge.id}
                      url={entry.badge.url}
                      fallbacks={entry.badge.fallbacks}
                      worn={entry.worn}
                      label={entry.badge.name || entry.badge.title || 'Badge'}
                      onClick={() => void equipSevenTvBadge(entry)}
                    />
                  )}
                </TileGrid>
                <ShowAll
                  count={ext.seventv_badges.length}
                  open={isOpen('seventvBadges')}
                  onToggle={() => toggleSection('seventvBadges')}
                />
              </>
            )}
          </>
        )}

        {/* Twitch badge */}
        {ext &&
          (ext.twitch_selectable.length > 0 ? (
            <>
              <Header title="Badge" brand="twitch" detail={ext.twitch_selectable.find((b) => b.is_selected)?.title} />
              <TileGrid items={ext.twitch_selectable} open={isOpen('twitch')} worn={(b) => b.is_selected}>
                {(badge) => (
                  <BadgeTile
                    key={`${badge.id}-${badge.version}`}
                    url={badge.image_url}
                    worn={badge.is_selected}
                    label={badge.title}
                    onClick={() => void equipTwitchBadge(badge)}
                  />
                )}
              </TileGrid>
              <ShowAll
                count={ext.twitch_selectable.length}
                open={isOpen('twitch')}
                onToggle={() => toggleSection('twitch')}
              />
            </>
          ) : (
            ext.twitch_earned.length > 0 && (
              <>
                <Header title="Badges" brand="twitch" />
                <div className="glass-panel p-3.5">
                  <div className="flex flex-wrap gap-2.5">
                    {ext.twitch_earned.map((badge, i) => (
                      <FallbackImage
                        key={`${badge.id}-${i}`}
                        src={badge.url}
                        fallbackUrls={badge.fallbacks}
                        alt={badge.title || ''}
                        title={badge.title}
                        className="w-7 h-7 object-contain"
                      />
                    ))}
                  </div>
                  {!ext.drops_signed_in && (
                    <div className="text-[12.5px] text-textSecondary mt-2.5">
                      Sign in to Drops to choose which badge you wear.
                    </div>
                  )}
                </div>
              </>
            )
          ))}

        {/* Name color */}
        {ext && ext.named_colors.length > 0 && (
          <>
            <Header title="Name color" brand="twitch" detail={currentColor?.label} />
            <div className="glass-panel p-3 grid grid-cols-8 gap-x-1.5 gap-y-3">
              {ext.named_colors.map((c) => {
                const on = c.hex.toLowerCase() === chatHex;
                return (
                  <button
                    key={c.value}
                    onClick={() => void setChatColor(c)}
                    aria-label={c.label}
                    aria-pressed={on}
                    className="h-9 flex items-center justify-center active:scale-90 transition-transform"
                  >
                    <span
                      className="block w-[30px] h-[30px] rounded-full"
                      style={{
                        backgroundColor: c.hex,
                        boxShadow: on
                          ? '0 0 0 2.5px var(--color-background), 0 0 0 4.5px var(--color-accent)'
                          : undefined,
                      }}
                    />
                  </button>
                );
              })}
            </div>
          </>
        )}
      </div>
    </DrillInScreen>
  );
};
