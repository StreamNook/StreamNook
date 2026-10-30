// The Discord profile card and member-list row, drawn from the activity Rust
// built for Discord (services/discord_presence.rs `PreviewModel`). Nothing
// here decides what the status says; it only lays it out the way Discord does.
// The colors are Discord's own dark theme: this is a picture of Discord.

import { useState } from 'react';
import { elapsedText } from './discordPreviewFormat';
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

export interface PreviewModel {
  activity: PreviewActivity | null;
  shown: PreviewTab;
  live: boolean;
  connection: 'off' | 'connected' | 'connecting' | 'waiting';
  user: { name: string; avatar_url: string } | null;
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

/** An image that steps aside (a neutral tile) when its URL does not load. */
function Picture({ src, className, alt }: { src: string | null; className: string; alt: string }) {
  const [failed, setFailed] = useState<string | null>(null);
  if (!src || failed === src) {
    return <div className={className} style={{ background: '#3f4147' }} aria-hidden />;
  }
  return <img src={src} alt={alt} className={className} onError={() => setFailed(src)} draggable={false} />;
}

export function DiscordStatusPreview({ model, now }: { model: PreviewModel | null; now: number }) {
  const name = model?.user?.name ?? 'You';
  const avatar = model?.user?.avatar_url ?? FALLBACK_AVATAR;
  const a = model?.activity ?? null;

  return (
    <div className="flex w-[340px] flex-col gap-3 select-none" style={{ color: TEXT }}>
      {/* The profile card a friend opens. */}
      <div className="overflow-hidden rounded-[12px] shadow-[0_8px_24px_rgba(0,0,0,0.35)]" style={{ background: CARD_BG }}>
        <div className="h-[60px]" style={{ background: 'linear-gradient(135deg,#5865f2,#3b42a8)' }} />
        <div className="relative px-4 pb-4">
          <div className="absolute -top-[40px] left-4 h-[80px] w-[80px] rounded-full p-[5px]" style={{ background: CARD_BG }}>
            <Picture src={avatar} alt="" className="h-full w-full rounded-full object-cover" />
            <span
              className="absolute bottom-[3px] right-[3px] h-[18px] w-[18px] rounded-full"
              style={{ background: ONLINE, boxShadow: `0 0 0 4px ${CARD_BG}` }}
              aria-hidden
            />
          </div>
          <div className="pt-[48px]">
            <div className="text-[18px] font-semibold leading-6">{name}</div>
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
      <div className="flex items-center gap-3 rounded-[8px] px-3 py-2" style={{ background: PANEL_BG }}>
        <div className="relative h-8 w-8 flex-shrink-0">
          <Picture src={avatar} alt="" className="h-full w-full rounded-full object-cover" />
          <span
            className="absolute -bottom-[1px] -right-[1px] h-[12px] w-[12px] rounded-full"
            style={{ background: ONLINE, boxShadow: `0 0 0 3px ${PANEL_BG}` }}
            aria-hidden
          />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[14px] font-medium leading-[18px]">{name}</div>
          <div className="truncate text-[12px] leading-4" style={{ color: MUTED }}>
            {a ? a.member_line : ''}
          </div>
        </div>
      </div>
    </div>
  );
}
