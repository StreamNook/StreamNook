// "What viewers see" in Stream info: the title, category and tags as typed,
// drawn with the app's own parts, so the preview is the real thing rather
// than a look-alike. The card is Home's live stream card (`glass-panel
// media-card`, the `CardChip` LIVE and viewer chips, the hover card's uptime
// chip); the row is the sidebar's row as `Sidebar.tsx` draws it. Offline, the
// card's preview well shows the category's box art over a blurred copy of
// itself and an Offline chip, since Twitch has no live picture to show.
import { Broadcast, UsersThree } from 'phosphor-react';
import StreamTitleWithEmojis from '../StreamTitleWithEmojis';
import { CardChip } from '../ui/CardChip';
import { formatStreamUptime } from '../../utils/chatCommands';
import { formatViewerCount } from '../../utils/streamStats';
import type { InfoCheck } from '../../services/streamManager';

type Preview = InfoCheck['preview'];

/** The partner mark the cards draw after a name. */
const PartnerMark = () => (
  <svg className="h-3 w-3 flex-shrink-0" viewBox="0 0 16 16" fill="#9146FF" aria-label="Verified">
    <path
      fillRule="evenodd"
      d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z"
      clipRule="evenodd"
    />
  </svg>
);

const Avatar = ({ url, px, className = '' }: { url: string | null; px: number; className?: string }) =>
  url ? (
    <img src={url} alt="" className={`flex-shrink-0 rounded-full object-cover ${className}`} style={{ width: px, height: px }} />
  ) : (
    <span className={`flex-shrink-0 rounded-full bg-textPrimary/10 ${className}`} style={{ width: px, height: px }} />
  );

/** The live picture with its chips, or the offline well. `status` keeps only
 *  the LIVE or Offline chip, for a well too small to carry the counts. */
function PreviewWell({ preview, status = false }: { preview: Preview; status?: boolean }) {
  const live = !!preview.thumbnail_url;
  const uptime = live && !status ? formatStreamUptime(preview.started_at ?? null) : '';
  return (
    <div className="relative w-full aspect-video overflow-hidden rounded bg-black/40">
      {live ? (
        <img src={preview.thumbnail_url ?? undefined} alt="" draggable={false} className="h-full w-full object-cover" />
      ) : preview.box_art_url ? (
        <>
          <img
            src={preview.box_art_url}
            alt=""
            aria-hidden
            draggable={false}
            className="absolute inset-0 h-full w-full scale-125 object-cover opacity-50 blur-xl"
          />
          <img
            src={preview.box_art_url}
            alt=""
            draggable={false}
            // In the small well the status chip takes the top left, so the
            // box art sits to the right of it instead of under it.
            className={`absolute top-1/2 h-[62%] -translate-y-1/2 rounded-sm shadow-lg ${
              status ? 'right-[14%]' : 'left-1/2 -translate-x-1/2'
            }`}
          />
        </>
      ) : null}
      <div className="absolute left-1.5 top-1.5 flex items-center gap-1">
        {live ? <CardChip kind="live">LIVE</CardChip> : <CardChip kind="neutral">Offline</CardChip>}
      </div>
      {live && !status && preview.viewer_count != null && (
        <CardChip kind="neutral" className="absolute bottom-1.5 left-1.5 tabular-nums">
          <UsersThree size={12} weight="bold" className="shrink-0 opacity-80" aria-label="viewers" />
          {preview.viewer_count.toLocaleString()}
        </CardChip>
      )}
      {uptime && (
        <CardChip kind="neutral" className="absolute bottom-1.5 right-1.5 tabular-nums">
          {uptime}
        </CardChip>
      )}
    </div>
  );
}

/** The title, channel and category lines as Home's card sets them, and the
 *  tags under them. */
function CardBody({ preview, tags }: { preview: Preview; tags: number }) {
  return (
    <div className="mt-2 min-w-0 space-y-0.5 pb-0.5">
      <h3 className="min-h-5 text-sm font-medium text-textPrimary line-clamp-1">
        {preview.title ? <StreamTitleWithEmojis title={preview.title} /> : <span className="text-textMuted">Your title</span>}
      </h3>
      <div className="flex min-w-0 items-center gap-1 text-xs text-textSecondary">
        <Avatar url={preview.avatar_url} px={16} className="ring-1 ring-borderSubtle" />
        <span className="truncate">{preview.channel_name}</span>
        {preview.partner && <PartnerMark />}
      </div>
      {preview.category_name && <p className="truncate text-xs text-textMuted">{preview.category_name}</p>}
      {tags > 0 && preview.tags.length > 0 && (
        <div className="flex flex-wrap gap-1 pt-1.5">
          {preview.tags.slice(0, tags).map((t) => (
            <span key={t} className="rounded-full bg-surface/60 px-2 py-px text-[11px] text-textSecondary">
              {t}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

/** Home's live stream card for this channel, as typed. */
export function ViewerCardPreview({ preview, tags = 4 }: { preview: Preview; tags?: number }) {
  return (
    <div className="glass-panel media-card p-2.5">
      <PreviewWell preview={preview} />
      <CardBody preview={preview} tags={tags} />
    </div>
  );
}

/** The card side by side, for the narrow panel above the composer: the same
 *  well (status chip only) and the same lines, no tags. */
export function CompactCardPreview({ preview }: { preview: Preview }) {
  return (
    <div className="glass-panel media-card flex gap-2.5 p-2">
      <div className="w-[128px] flex-shrink-0">
        <PreviewWell preview={preview} status />
      </div>
      <div className="-mt-2 min-w-0 flex-1">
        <CardBody preview={preview} tags={0} />
      </div>
    </div>
  );
}

/** The sidebar's row for this channel while it is live, as `Sidebar.tsx`
 *  draws it expanded: picture with the live dot, name, category, and the
 *  count at the right. The sidebar lists only live channels, so offline it
 *  still shows the live row (without a count), as it will look once live. */
export function SidebarRowPreview({ preview }: { preview: Preview }) {
  const live = !!preview.thumbnail_url;
  return (
    <div className="flex items-center gap-2 rounded border-l-2 border-transparent px-2 py-1.5">
      <div className="relative flex-shrink-0">
        <Avatar url={preview.avatar_url} px={32} />
        <div className="absolute -bottom-0.5 -right-0.5 h-3 w-3 rounded-full border-2 border-background bg-live" />
      </div>
      <div className="min-w-0 flex-1 overflow-hidden">
        <div className="flex items-center gap-1">
          <span className="truncate text-sm font-medium text-textPrimary">{preview.channel_name}</span>
          {preview.partner && <PartnerMark />}
        </div>
        <div className="truncate text-xs text-textMuted">{preview.category_name || 'Just Chatting'}</div>
      </div>
      {live && preview.viewer_count != null && (
        <div className="flex h-5 flex-shrink-0 items-center gap-1 self-start text-xs text-textSecondary">
          <Broadcast size={10} weight="bold" className="text-live" />
          <span>{formatViewerCount(preview.viewer_count)}</span>
        </div>
      )}
    </div>
  );
}
