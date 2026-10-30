// The Discord profile card and member-list row, drawn from the activity Rust
// built for Discord (services/discord_presence.rs `PreviewModel`) and the
// viewer's own Discord look (services/discord_look.rs). Nothing here decides
// what the status says or which picture is shown; it only lays them out the
// way Discord does and picks the moving or still frame (discordPreviewLook).
// The colors are Discord's own dark theme: this is a picture of Discord.

import { useState } from 'react';
import { useAppStore } from '../../../stores/AppStore';
import { IS_MAC } from '../../../utils/platform';
import { elapsedText } from './discordPreviewFormat';
import { animateCard, animateRow, pick, plateVideo, type LookPair } from './discordPreviewLook';
import { Tooltip } from '../../ui/Tooltip';

export type PreviewTab = 'live' | 'replay' | 'grid' | 'offline' | 'browsing';

export interface PreviewButton {
  label: string;
  url: string;
}

export interface PreviewActivity {
  verb: string;
  app_name: string;
  member_line: string;
  details: string | null;
  details_url: string | null;
  state: string | null;
  large_image: string;
  large_text: string | null;
  small_image: string | null;
  small_text: string | null;
  start: number | null;
  buttons: PreviewButton[];
}

export interface PreviewUser {
  id: string;
  name: string;
  username: string;
  avatar_url: string;
  avatar_still: string;
  decoration: string | null;
  decoration_still: string | null;
}

export interface PreviewNameplate {
  image: string;
  video: string | null;
  palette: string | null;
}

export interface PreviewServerTag {
  tag: string;
  badge: string | null;
}

/** The viewer's Discord look, resolved by Rust. */
export interface PreviewLook {
  banner: LookPair | null;
  /** The banner area's color when there is no banner picture. */
  strip: string | null;
  decoration: LookPair | null;
  nameplate: PreviewNameplate | null;
  server_tag: PreviewServerTag | null;
}

export interface PreviewModel {
  activity: PreviewActivity | null;
  shown: PreviewTab;
  live: boolean;
  connection: 'off' | 'connected' | 'connecting' | 'waiting';
  user: PreviewUser | null;
  look: PreviewLook | null;
  tokens: { token: string; value: string }[];
  /** `fields` holds only what the preset sets (never browsing or the promo button). */
  presets: { id: string; label: string; blurb: string; fields: Record<string, unknown> }[];
}

const CARD_BG = '#232428';
const PANEL_BG = '#2b2d31';
const TEXT = '#f2f3f5';
const MUTED = '#b5bac1';
const BUTTON_BG = '#4e5058';
const ONLINE = '#23a55a';
const FALLBACK_AVATAR = 'https://cdn.discordapp.com/embed/avatars/0.png';
const DEFAULT_BANNER = 'linear-gradient(135deg,#5865f2,#3b42a8)';

// Discord's nameplate palettes, as the tint under the art.
const PALETTE: Record<string, string> = {
  crimson: '144, 0, 7',
  berry: '137, 58, 153',
  sky: '0, 128, 183',
  teal: '8, 100, 96',
  forest: '45, 84, 1',
  bubble_gum: '220, 62, 151',
  violet: '115, 11, 200',
  cobalt: '1, 49, 194',
  clover: '4, 123, 32',
  lemon: '246, 205, 18',
  white: '255, 255, 255',
};

/** An image that steps aside (a neutral tile) when its URL does not load. */
function Picture({ src, className, alt }: { src: string | null; className: string; alt: string }) {
  const [failed, setFailed] = useState<string | null>(null);
  if (!src || failed === src) {
    return <div className={className} style={{ background: '#3f4147' }} aria-hidden />;
  }
  return <img src={src} alt={alt} className={className} onError={() => setFailed(src)} draggable={false} />;
}

/** A picture over another that simply goes away when it does not load. */
function Overlay({ src, className }: { src: string | null; className: string }) {
  const [failed, setFailed] = useState<string | null>(null);
  if (!src || failed === src) return null;
  return <img src={src} alt="" aria-hidden className={className} onError={() => setFailed(src)} draggable={false} />;
}

/** The banner area: one fixed height in every state, its color under a
 *  picture that fades in when it has loaded. */
function Banner({ look, animate }: { look: PreviewLook | null; animate: boolean }) {
  const src = pick(look?.banner, animate);
  const [loaded, setLoaded] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  return (
    <div className="relative h-[100px] overflow-hidden" style={{ background: look?.strip ?? DEFAULT_BANNER }}>
      {src && failed !== src && (
        <img
          src={src}
          alt=""
          aria-hidden
          draggable={false}
          onLoad={() => setLoaded(src)}
          onError={() => setFailed(src)}
          className="absolute inset-0 h-full w-full object-cover transition-opacity duration-300"
          style={{ opacity: loaded === src ? 1 : 0 }}
        />
      )}
    </div>
  );
}

/** The nameplate behind the member-list row, as Discord draws it: the
 *  palette's tint from the left, the art in the right 60% fading in. */
function Nameplate({ plate, animate }: { plate: PreviewNameplate; animate: boolean }) {
  const rgb = (plate.palette && PALETTE[plate.palette]) || '151, 177, 185';
  const video = plateVideo(plate.video, animate, IS_MAC);
  const fade = {
    WebkitMaskImage: 'linear-gradient(to right, transparent 0, #000 45%)',
    maskImage: 'linear-gradient(to right, transparent 0, #000 45%)',
  };
  return (
    <span
      aria-hidden
      className="pointer-events-none absolute inset-0 overflow-hidden rounded-[inherit]"
      style={{ background: `linear-gradient(90deg, rgba(${rgb}, 0), rgba(${rgb}, 0.18))` }}
    >
      {video ? (
        <video
          src={video}
          poster={plate.image}
          autoPlay
          muted
          loop
          playsInline
          className="absolute inset-y-0 right-0 h-full w-[60%] object-cover object-right"
          style={fade}
        />
      ) : (
        <Overlay src={plate.image} className="absolute inset-y-0 right-0 h-full w-[60%] object-cover object-right" />
      )}
    </span>
  );
}

/** The server tag the viewer wears, after the name. */
function ServerTag({ tag }: { tag: PreviewServerTag }) {
  return (
    <span className="inline-flex h-[18px] flex-none items-center gap-1 rounded-[5px] bg-[rgba(8,8,12,0.6)] px-1.5 text-[11px] font-semibold leading-4 shadow-[inset_0_0_0_1px_rgba(255,255,255,0.1)]">
      <Overlay src={tag.badge} className="h-3 w-3 flex-none object-contain" />
      {tag.tag}
    </span>
  );
}

export function DiscordStatusPreview({ model, now }: { model: PreviewModel | null; now: number }) {
  const motion = useAppStore((s) => s.settings?.motion_mode);
  const [rowHovered, setRowHovered] = useState(false);
  const user = model?.user ?? null;
  const look = model?.look ?? null;
  const card = animateCard(motion);
  const row = animateRow(motion, rowHovered);
  const name = user?.name ?? 'You';
  const avatarPair: LookPair | null = user ? { animated: user.avatar_url, still: user.avatar_still } : null;
  const cardAvatar = pick(avatarPair, card) ?? FALLBACK_AVATAR;
  const rowAvatar = pick(avatarPair, row) ?? FALLBACK_AVATAR;
  const tag = look?.server_tag ?? null;
  const a = model?.activity ?? null;

  return (
    <div className="flex w-[340px] flex-col gap-3 select-none" style={{ color: TEXT }}>
      {/* The profile card a friend opens. */}
      <div className="overflow-hidden rounded-[12px] shadow-[0_8px_24px_rgba(0,0,0,0.35)]" style={{ background: CARD_BG }}>
        <Banner look={look} animate={card} />
        <div className="relative px-4 pb-4">
          <div className="absolute -top-[40px] left-4 h-[80px] w-[80px] rounded-full p-[5px]" style={{ background: CARD_BG }}>
            <Picture src={cardAvatar} alt="" className="h-full w-full rounded-full object-cover" />
            <Overlay
              src={pick(look?.decoration, card)}
              className="pointer-events-none absolute left-1/2 top-1/2 h-[96px] w-[96px] max-w-none -translate-x-1/2 -translate-y-1/2"
            />
            <span
              className="absolute bottom-[3px] right-[3px] h-[18px] w-[18px] rounded-full"
              style={{ background: ONLINE, boxShadow: `0 0 0 4px ${CARD_BG}` }}
              aria-hidden
            />
          </div>
          <div className="pt-[48px]">
            <div className="flex min-w-0 items-center gap-1.5">
              <span className="truncate text-[18px] font-semibold leading-6">{name}</span>
              {tag && <ServerTag tag={tag} />}
            </div>
            {/* Always there, so the card never grows when Discord connects. */}
            <div className="min-h-[18px] truncate text-[13px] leading-[18px]" style={{ color: MUTED }}>
              {user?.username ? `@${user.username}` : ''}
            </div>
          </div>

          <div className="mt-3 rounded-[8px] p-3" style={{ background: PANEL_BG }}>
            {a ? (
              <>
                <div className="mb-2 text-[12px] font-semibold" style={{ color: MUTED }}>
                  {a.verb}
                </div>
                <div className="flex items-start gap-3">
                  <div className="relative h-[72px] w-[72px] flex-shrink-0">
                    <Picture src={a.large_image} alt={a.large_text ?? ''} className="h-full w-full rounded-[8px] object-cover" />
                    {a.small_image && (
                      <Tooltip content={a.small_text ?? ''} disabled={!a.small_text}>
                        <div
                          className="absolute -bottom-[4px] -right-[4px] h-[26px] w-[26px] rounded-full p-[3px]"
                          style={{ background: PANEL_BG }}
                        >
                          <Picture src={a.small_image} alt={a.small_text ?? ''} className="h-full w-full rounded-full object-cover" />
                        </div>
                      </Tooltip>
                    )}
                  </div>
                  <div className="min-w-0 flex-1 text-[13px] leading-[18px]">
                    <div className="truncate font-semibold">{a.app_name}</div>
                    {a.details && (
                      <div className={`truncate ${a.details_url ? 'underline-offset-2 hover:underline' : ''}`}>{a.details}</div>
                    )}
                    {a.state && <div className="truncate">{a.state}</div>}
                    {a.start != null && (
                      <div className="mt-0.5 text-[12px] tabular-nums" style={{ color: ONLINE }}>
                        {elapsedText(a.start, now)}
                      </div>
                    )}
                  </div>
                </div>
                {a.buttons.length > 0 && (
                  <div className="mt-3 flex flex-col gap-2">
                    {a.buttons.map((b, i) => (
                      <div
                        key={i}
                        className="flex h-8 items-center justify-center truncate rounded-[4px] px-3 text-[13px] font-medium"
                        style={{ background: BUTTON_BG }}
                      >
                        {b.label}
                      </div>
                    ))}
                  </div>
                )}
              </>
            ) : (
              <div className="py-3 text-center text-[13px]" style={{ color: MUTED }}>
                No status while browsing
              </div>
            )}
          </div>
        </div>
      </div>

      {/* The member list row everyone in a server sees. */}
      <div
        className="relative flex items-center gap-3 overflow-hidden rounded-[8px] px-3 py-2"
        style={{ background: PANEL_BG }}
        onMouseEnter={() => setRowHovered(true)}
        onMouseLeave={() => setRowHovered(false)}
      >
        {look?.nameplate && <Nameplate plate={look.nameplate} animate={row} />}
        <div className="relative h-8 w-8 flex-shrink-0">
          <Picture src={rowAvatar} alt="" className="h-full w-full rounded-full object-cover" />
          <Overlay
            src={pick(look?.decoration, row)}
            className="pointer-events-none absolute left-1/2 top-1/2 h-[40px] w-[40px] max-w-none -translate-x-1/2 -translate-y-1/2"
          />
          <span
            className="absolute -bottom-[1px] -right-[1px] h-[12px] w-[12px] rounded-full"
            style={{ background: ONLINE, boxShadow: `0 0 0 3px ${PANEL_BG}` }}
            aria-hidden
          />
        </div>
        <div className="relative min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="truncate text-[14px] font-medium leading-[18px]">{name}</span>
            {tag && <ServerTag tag={tag} />}
          </div>
          <div className="truncate text-[12px] leading-4" style={{ color: MUTED }}>
            {a ? a.member_line : ''}
          </div>
        </div>
      </div>
    </div>
  );
}
