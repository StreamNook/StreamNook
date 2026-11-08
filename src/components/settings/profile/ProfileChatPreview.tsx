// How you read in a chat line: the badges you wear drawn by chat's own badge
// renderer (its order, its +N fold at your limit), your painted name, and the
// look being hovered in Customize (or the one you wear). It reads the hovered
// look from its own small store, so the pointer crossing the picker rows
// repaints this line alone.
import { useMemo, type ReactNode } from 'react';
import { useFreshIds } from '../../../hooks/useFreshIds';
import { ExternalLink } from 'lucide-react';
import { AtmosphereBackground } from '../../AtmosphereBackground';
import { ChatBadgeIcons, type ResolvedTwitchBadge, type SevenTvBadgeLike } from '../../chat/ChatBadgeIcons';
import { Tooltip } from '../../ui/Tooltip';
import { computePaintStyle } from '../../../services/seventvService';
import { getAtmosphere, type Atmosphere } from '../../../services/atmospheres';
import type { ThirdPartyBadge } from '../../../services/badgeService';
import { previewEmoteUrl, rollPreviewChat, type PreviewEmote } from '../../../utils/previewChat';
import { badgeFoldLimit } from '../../../utils/badgeFold';
import { useAppStore } from '../../../stores/AppStore';
import { features } from '../../../features';
import type { SevenTVPaint } from '../../../types';
import type { ChatIdentityBadge } from '../../../types/profileView';
import { useHoveredTheme } from './profilePreview';

interface Props {
  profileTheme: string;
  chatAtmosphere: boolean;
  canPaint: boolean;
  atmUnlocked: (a: Atmosphere) => boolean;
  onGetSupport: (tier: 'supporter' | 'subscriber') => void;
  /** Your user id while you are a StreamNook member (draws your badge). */
  memberId: string | null;
  /** The StreamNook badge design you wear, so changing it plays the swap. */
  memberBadge: string | null | undefined;
  /** The Twitch global badge you wear. */
  twitchBadge: ChatIdentityBadge | undefined;
  seventvBadge: SevenTvBadgeLike | null | undefined;
  /** The chat-client badges you show. */
  thirdParty: readonly ThirdPartyBadge[];
  displayName: string;
  paint: SevenTVPaint | null;
  emotes: PreviewEmote[];
}

export function ProfileChatPreview({
  profileTheme,
  chatAtmosphere,
  canPaint,
  atmUnlocked,
  onGetSupport,
  memberId,
  memberBadge,
  twitchBadge,
  seventvBadge,
  thirdParty,
  displayName,
  paint,
  emotes,
}: Props) {
  const hovered = useHoveredTheme();
  // What you wear, one id per cosmetic, so the piece you just picked plays its
  // swap and the rest hold still. The ids match ChatBadgeIcons' slot ids, with
  // the StreamNook slot carrying its design.
  const snId = memberId ? `sn-${memberBadge ?? ''}` : null;
  const paintId = `paint-${paint?.id ?? 'none'}`;
  const fresh = useFreshIds([
    twitchBadge ? `tw-${twitchBadge.id}/${twitchBadge.version}` : '',
    seventvBadge ? `7tv-${seventvBadge.id}` : '',
    ...thirdParty.map((b) => `tp-${b.provider}-${b.id}`),
    snId ?? '',
    paintId,
  ].filter(Boolean));
  const wrapSlot = (id: string, node: ReactNode) => {
    const key = id === 'sn' ? (snId ?? id) : id;
    return (
      <span key={key} className={fresh.has(key) ? 'sn-cosmetic-swap' : 'inline-flex'}>
        {node}
      </span>
    );
  };
  // Your fold limit from Chat settings, so the line folds like your chat does.
  const foldLimit = badgeFoldLimit(useAppStore((s) => s.settings?.chat_design?.badge_fold_limit));
  // The worn Twitch badge in the shape chat rows carry; the key is the one the
  // badge art cache uses, so the line draws the disk copy chat would.
  const twitchBadges = useMemo<ResolvedTwitchBadge[]>(
    () =>
      twitchBadge
        ? [
            {
              key: `${twitchBadge.id}/${twitchBadge.version}`,
              info: {
                id: twitchBadge.version,
                title: twitchBadge.title,
                image_url_1x: twitchBadge.image_url,
                image_url_4x: twitchBadge.image_url,
              },
            },
          ]
        : [],
    [twitchBadge],
  );
  const activeId = hovered ?? profileTheme;
  const atm = getAtmosphere(activeId);
  // The preview shows what chat will show, so it drops the wash when the
  // member keeps their atmosphere out of chat. Busy washes (Midnight) get the
  // frosted readability block real chat uses.
  const chatAtm = chatAtmosphere ? atm : null;
  const frost = !!chatAtm?.chatFrost;
  const locked = atm ? !atmUnlocked(atm) : activeId === 'paint' ? !canPaint : false;
  // An Atmosphere needs Subscriber; a 7TV paint background only Supporter.
  const lockTier: 'supporter' | 'subscriber' = atm ? 'subscriber' : 'supporter';
  // A fresh sample line each time the previewed look changes.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const line = useMemo(() => rollPreviewChat(emotes), [activeId, emotes]);

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2 px-1">
        <span className="text-[10px] font-semibold uppercase tracking-[0.12em] text-textMuted">Chat preview</span>
        {locked && (
          <button
            type="button"
            onClick={() => onGetSupport(lockTier)}
            className="inline-flex items-center gap-1 rounded-full border border-amber-400/30 bg-amber-400/15 px-2 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-amber-200/90 transition-colors hover:bg-amber-400/25"
          >
            Get {lockTier === 'supporter' ? 'Supporter' : 'Subscriber'}
            <ExternalLink size={9} className="opacity-70" />
          </button>
        )}
      </div>

      {/* Real chat sizes, capped at a real chat column (~402px). Inline flow so
          a long message wraps to the left edge like a real row; the min height
          gives an atmosphere two lines to read in. */}
      <div
        className="relative isolate w-full max-w-[402px] overflow-hidden rounded-lg border border-white/[0.06]"
        style={{ paddingLeft: 18, paddingRight: 18, paddingTop: 7, paddingBottom: 7, minHeight: 60 }}
      >
        {chatAtm && <AtmosphereBackground atm={chatAtm} variant="chat" />}
        <div
          className={`relative text-sm leading-relaxed ${
            frost
              ? `inline-block max-w-[calc(100%+0.75rem)] rounded-md px-1.5 -mx-1.5 py-0.5 ${features.richAtmospheres ? 'atm-frost' : 'atm-frost atm-frost--flat'}`
              : ''
          }`}
        >
          <span className="mr-1.5 align-middle text-[11px] text-textMuted">3:45</span>
          <span className="sn-badge-strip mr-1.5">
            <ChatBadgeIcons
              twitchBadges={twitchBadges}
              seventvBadge={seventvBadge}
              thirdPartyBadges={thirdParty}
              streamNookMemberId={memberId}
              foldLimit={foldLimit}
              keyPrefix="profile-preview"
              wrapSlot={wrapSlot}
            />
          </span>
          <span
            key={paintId}
            className={`align-middle font-semibold ${fresh.has(paintId) ? 'sn-cosmetic-swap sn-cosmetic-swap--name' : ''}`}
            style={paint ? computePaintStyle(paint as never, '#9146FF') : undefined}
          >
            {displayName}
          </span>
          <span className="align-middle text-textSecondary">
            {' '}
            {line.text}
            {line.emotes.map((e, i) => (
              <Tooltip key={`${e.id}-${i}`} content={e.name}>
                <img
                  src={previewEmoteUrl(e.id)}
                  alt={e.name}
                  className="mx-0.5 inline-block h-6 w-auto align-middle object-contain"
                  onError={(ev) => {
                    (ev.currentTarget as HTMLImageElement).style.display = 'none';
                  }}
                />
              </Tooltip>
            ))}
          </span>
        </div>
      </div>
    </div>
  );
}
