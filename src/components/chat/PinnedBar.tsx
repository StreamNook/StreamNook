// The pinned messages at the top of chat: the open card (every pin, with its
// sender drawn as a chat row draws them, link cards and who pinned it) or the
// one-line bar it folds to. Chat (ChatWidget) owns the pins, the fold and the
// unpin; Settings > Chat's preview draws the same component with a sample pin.
import { AnimatePresence, motion } from 'framer-motion';
import type { ReactNode } from 'react';
import { CaretDown } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import type { ProviderId } from '../../types/providers';
import type { MessageSegment } from '../../services/twitchChat';
import { previewUrlsFromLinks, type RowLink } from '../../services/linkPreviewService';
import { LinkPreviewCard } from './LinkPreviewCard';
import { PinnedChatter } from './PinnedChatter';
import { SevenTvAvatar } from '../SevenTvAvatar';
import { Tooltip } from '../ui/Tooltip';

export interface PinnedMessage {
  id: string;
  message_id: string;
  type: string;
  message_text: string;
  sender_id: string;
  sender_name: string;
  sender_login?: string;
  sender_shown_name?: string;
  sender_color: string;
  sender_avatar: string;
  sender_badges: Array<{ set_id: string; version: string }>;
  pinned_by: string;
  pinned_by_id: string;
  pinned_by_avatar: string;
  started_at: string;
  /** The text as text and link segments, split in Rust (`text_spans::pin_segments`). */
  segments?: MessageSegment[];
}

// Lifts an unpainted pinned name off the glass; a 7TV paint brings its own shadow.
const PIN_NAME_SHADOW = { textShadow: '0 1px 2px rgba(0,0,0,0.8)' } as const;

interface PinnedBarProps {
  pins: PinnedMessage[];
  expanded: boolean;
  /** Open the card; `pinId` is the newest pin, now seen. */
  onExpand: (pinId: string) => void;
  onCollapse: () => void;
  provider: ProviderId;
  /** The channel's id, for its badges. */
  channelId?: string;
  /** Where it floats from the top of the chat column. */
  top: number;
  hidden?: boolean;
  /** Unpin (moderators): absent hides the button. */
  onUnpin?: (messageId: string) => void;
  onBadgeClick?: (badgeKey: string, badgeInfo: { url?: string; image_url_4x?: string }) => void;
  /** A pin's text with the channel's emotes, as chat draws it. */
  renderText: (text: string, keyPrefix: string, emoteClass: string) => ReactNode;
}

export function PinnedBar({
  pins,
  expanded,
  onExpand,
  onCollapse,
  provider,
  channelId,
  top,
  hidden = false,
  onUnpin,
  onBadgeClick,
  renderText,
}: PinnedBarProps) {
  const design = useAppStore((s) => s.settings.chat_design);
  if (pins.length === 0) return null;
  return (
    <div
      className={`absolute left-3 right-3 z-[15] pointer-events-none flex flex-col items-center ${hidden ? 'hidden' : ''}`}
      style={{ top }}
    >
      <AnimatePresence>
        {expanded && (
          <motion.div
            initial={{ opacity: 0, y: -10, scale: 0.96, height: 0 }}
            animate={{ opacity: 1, y: 0, scale: 1, height: 'auto' }}
            exit={{ opacity: 0, y: -10, scale: 0.96, height: 0 }}
            transition={{ duration: 0.3, ease: [0.175, 0.885, 0.32, 1.2] }}
            // The emote, bits and mod menus' glass, blur and corners.
            className="glass-flyout rounded-xl !p-0 w-full max-w-sm mt-2 pointer-events-auto overflow-hidden origin-top cursor-pointer"
            // Clicking the open pin folds it back to the bar, the same gesture
            // that opened it. Interactive children (unpin, links, preview cards)
            // keep their own click, and a drag that selected text is a copy,
            // not a collapse.
            role="button"
            tabIndex={0}
            aria-expanded={true}
            aria-label="Collapse pinned message"
            onClick={(e) => {
              const target = e.target as HTMLElement;
              if (target.closest('button, a, input, textarea, select, video, [role="button"]:not([data-pin-card]), [data-no-pin-toggle]')) return;
              if ((window.getSelection()?.toString() ?? '').length > 0) return;
              onCollapse();
            }}
            onKeyDown={(e) => {
              if (e.target !== e.currentTarget) return;
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onCollapse();
              }
            }}
            data-pin-card=""
          >
            {pins.map((pin, i) => {
              // Same link-preview pipeline the main chat uses, applied to the
              // pinned message.
              const previewsOn = design?.link_previews ?? true;
              const keepLink = design?.link_preview_keep_link ?? false;
              const shortenLinks = design?.shorten_links ?? true;
              // Rust split the text into text and links, as it does a chat row.
              const pinParts: MessageSegment[] = pin.segments ?? [{ type: 'text', content: pin.message_text }];
              const pinLinks: RowLink[] = [];
              for (const seg of pinParts) if (seg.type === 'link') pinLinks.push(seg);
              const previewItems = previewsOn ? previewUrlsFromLinks(pinLinks, 2, design?.link_preview_trusted_domains) : [];
              // In "clean" mode the inline link is suppressed for any trusted
              // (auto-carded) URL so the card is the sole representation,
              // matching chat. Untrusted links keep their inline link.
              const suppressed = !keepLink ? new Set(previewItems.filter((it) => it.trusted).map((it) => it.url)) : null;
              const hasVisibleBody = pinParts.some((seg) =>
                seg.type === 'link' ? !suppressed?.has(seg.url) : seg.content.trim().length > 0,
              );
              return (
                <div key={pin.id} className={`relative p-3.5 ${i > 0 ? 'border-t border-white/[0.05]' : ''}`}>
                  {/* Unpin (mod-only): clears the current mod pin from the
                      pinned message itself. */}
                  {onUnpin && (
                    <Tooltip content="Unpin message" side="left">
                      <button
                        type="button"
                        onClick={() => onUnpin(pin.message_id)}
                        className="absolute top-2 right-2 z-10 p-1.5 rounded-lg text-textSecondary/70 hover:text-red-400 hover:bg-red-500/15 transition-colors pointer-events-auto"
                        aria-label="Unpin message"
                      >
                        <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><line x1="2" x2="22" y1="2" y2="22"/><line x1="12" x2="12" y1="17" y2="22"/><path d="M9 9v1.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V17h12"/><path d="M15 9.34V6h1a2 2 0 0 0 0-4H7.89"/></svg>
                      </button>
                    </Tooltip>
                  )}
                  {/* Sender row: avatar + badges + name + message */}
                  <div className="flex items-start gap-3">
                    {/* Sender avatar */}
                    {pin.sender_avatar ? (
                      <SevenTvAvatar
                        cosmeticsId={provider === 'twitch' ? pin.sender_id : `${provider}:${pin.sender_id}`}
                        src={pin.sender_avatar}
                        px={36}
                        motion="animate"
                        alt={pin.sender_name}
                        className="w-9 h-9 rounded-full flex-shrink-0 object-cover border border-white/5"
                      />
                    ) : (
                      <div
                        className="w-9 h-9 rounded-full flex-shrink-0 flex items-center justify-center text-[13px] font-bold text-white shadow-inner"
                        style={{ backgroundColor: pin.sender_color }}
                      >
                        {pin.sender_name.charAt(0).toUpperCase()}
                      </div>
                    )}
                    <div className="flex-1 min-w-0 pt-0.5">
                      {/* Name + badges row */}
                      <div className="flex items-center gap-1.5 flex-wrap">
                        {/* Badges and name exactly as a chat row draws them:
                            chat-order badges, 7TV, third-party, StreamNook,
                            then the name in its paint. */}
                        <PinnedChatter
                          variant="card"
                          provider={provider}
                          userId={pin.sender_id}
                          name={pin.sender_name}
                          login={pin.sender_login}
                          shownName={pin.sender_shown_name}
                          color={pin.sender_color}
                          badges={pin.sender_badges}
                          channelId={channelId}
                          onBadgeClick={onBadgeClick}
                          nameClassName="text-sm font-bold tracking-tight"
                          plainNameStyle={PIN_NAME_SHADOW}
                        />
                      </div>
                      {/* Message text with clickable links. Trusted carded
                          links are suppressed in clean mode (the card is the
                          link); the whole paragraph is skipped if nothing
                          visible remains (e.g. a pin that's only a link). */}
                      {hasVisibleBody && (
                        <p className="text-[13.5px] text-textPrimary/95 mt-1.5 break-words font-medium" style={{ lineHeight: '1.5' }}>
                          {pinParts.map((seg, index) =>
                            seg.type === 'link' ? (
                              suppressed?.has(seg.url) ? null : (
                                <a
                                  key={index}
                                  className="text-accent/90 hover:text-accent hover:underline pointer-events-auto cursor-pointer transition-colors"
                                  onClick={() => {
                                    import('@tauri-apps/plugin-shell').then(({ open }) => open(seg.url));
                                  }}
                                >
                                  {shortenLinks && seg.label ? seg.label : seg.content}
                                </a>
                              )
                            ) : (
                              <span key={index}>{renderText(seg.content, `pin-${pin.id}-${index}`, 'h-6 max-w-[112px] -my-1')}</span>
                            ),
                          )}
                        </p>
                      )}
                      {/* Inline link preview cards. In clean mode the inline
                          link above is suppressed and showChip lets a failed
                          trusted preview fall back to a link chip so the link is
                          never lost. Untrusted links render a click-to-load
                          chip (with the always-trust shield). */}
                      {previewItems.length > 0 && (
                        <div className="mt-2 flex flex-col items-start gap-1">
                          {previewItems.map((it) => (
                            <LinkPreviewCard key={it.url} url={it.url} trusted={it.trusted} showChip={it.trusted && !keepLink} />
                          ))}
                        </div>
                      )}
                    </div>
                  </div>
                  {/* Pinned by footer — visually separated */}
                  {pin.pinned_by && (
                    <div className="flex items-center gap-2 mt-3 pt-2.5 border-t border-white/[0.04] ml-[48px]">
                      <svg className="w-3 h-3 text-accent" fill="currentColor" viewBox="0 0 16 16">
                        <path d="M4.146.146A.5.5 0 0 1 4.5 0h7a.5.5 0 0 1 .5.5c0 .68-.342 1.174-.646 1.479-.126.125-.25.224-.354.298v4.431l.078.048c.203.127.476.314.751.555C12.36 7.775 13 8.527 13 9.5a.5.5 0 0 1-.5.5h-4v4.5a.5.5 0 0 1-1 0V10h-4A.5.5 0 0 1 3 9.5c0-.973.64-1.725 1.17-2.189A5.921 5.921 0 0 1 5 6.708V2.277a2.77 2.77 0 0 1-.354-.298C4.342 1.674 4 1.179 4 .5a.5.5 0 0 1 .146-.354z"/>
                      </svg>
                      {pin.pinned_by_avatar && (
                        <SevenTvAvatar
                          cosmeticsId={provider === 'twitch' ? pin.pinned_by_id : `${provider}:${pin.pinned_by_id}`}
                          src={pin.pinned_by_avatar}
                          px={16}
                          alt={pin.pinned_by}
                          className="w-4 h-4 rounded-full object-cover border border-white/10"
                        />
                      )}
                      <span className="text-[11px] text-textSecondary uppercase tracking-wide font-medium">
                        Pinned by{' '}
                        <span className="inline-flex items-center gap-1 align-middle ml-0.5">
                          <PinnedChatter
                            variant="line"
                            provider={provider}
                            userId={pin.pinned_by_id}
                            name={pin.pinned_by}
                            nameClassName="font-bold text-textPrimary/80"
                          />
                        </span>
                      </span>
                    </div>
                  )}
                </div>
              );
            })}
          </motion.div>
        )}
      </AnimatePresence>
      {/* Collapsed state: a thin one-line bar (pin + sender + truncated text)
          instead of hiding the pin entirely. Click to expand. Opt-out via
          chat_design.pinned_collapsed_style === 'hidden'. */}
      <AnimatePresence>
        {!expanded && (design?.pinned_collapsed_style ?? 'bar') !== 'hidden' && (() => {
          const pin = pins[0];
          return (
            <motion.button
              type="button"
              initial={{ opacity: 0, y: -8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
              onClick={() => onExpand(pin.id)}
              className="glass-flyout rounded-xl group w-full max-w-sm mt-2 pointer-events-auto overflow-hidden flex items-center gap-2.5 !px-3.5 !py-2.5 text-left transition-colors"
            >
              <svg className="w-3.5 h-3.5 text-accent flex-shrink-0" fill="currentColor" viewBox="0 0 16 16">
                <path d="M4.146.146A.5.5 0 0 1 4.5 0h7a.5.5 0 0 1 .5.5c0 .68-.342 1.174-.646 1.479-.126.125-.25.224-.354.298v4.431l.078.048c.203.127.476.314.751.555C12.36 7.775 13 8.527 13 9.5a.5.5 0 0 1-.5.5h-4v4.5a.5.5 0 0 1-1 0V10h-4A.5.5 0 0 1 3 9.5c0-.973.64-1.725 1.17-2.189A5.921 5.921 0 0 1 5 6.708V2.277a2.77 2.77 0 0 1-.354-.298C4.342 1.674 4 1.179 4 .5a.5.5 0 0 1 .146-.354z"/>
              </svg>
              {/* Sender: the name in its paint and at most the role badge, so
                  the name and preview keep the line. The cap only stops a very
                  long name from eating the preview. */}
              <span className="text-[13px] flex items-center gap-1.5 min-w-0 max-w-[50%] shrink">
                <PinnedChatter
                  variant="bar"
                  provider={provider}
                  userId={pin.sender_id}
                  name={pin.sender_name}
                  login={pin.sender_login}
                  shownName={pin.sender_shown_name}
                  color={pin.sender_color}
                  badges={pin.sender_badges}
                  channelId={channelId}
                  nameClassName="font-bold min-w-0 truncate"
                  plainNameStyle={PIN_NAME_SHADOW}
                />
              </span>
              <span className="text-[13px] text-textPrimary/85 truncate min-w-0 flex-1">
                {renderText(pin.message_text, `pinbar-${pin.id}`, 'h-5 max-w-[80px] -my-0.5')}
              </span>
              {pins.length > 1 && (
                <span className="text-[11px] font-semibold text-textSecondary flex-shrink-0 tabular-nums">+{pins.length - 1}</span>
              )}
              {/* Expand chevron */}
              <CaretDown size={12} weight="bold" aria-hidden className="text-textSecondary/60 group-hover:text-textSecondary flex-shrink-0 transition-colors" />
            </motion.button>
          );
        })()}
      </AnimatePresence>
    </div>
  );
}
