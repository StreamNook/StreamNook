// Settings > Chat preview's message box, built from the parts chat's composer
// uses (chat/composerParts, EmoteSmileyButton, WalletFace, RoomModeIndicator,
// the drops bar, the Tab completion views) so every Chat Input, Tab Completion
// and Drops setting shows as chat shows it. Nothing here sends or opens
// anything; the field is read-only.
import { useEffect, useRef, useState } from 'react';
import { AnimatePresence } from 'framer-motion';
import { PaperPlaneTilt } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { composerInputLook, composerLook, composerPaddingLeft, sendButtonClass } from '../chat/composerParts';
import { EmoteSmileyButton } from '../chat/EmoteSmileyButton';
import { RoomModeIndicator } from '../chat/RoomModeIndicator';
import { WalletFace, type WalletPoints } from '../chat/ChannelWallet';
import { DropsChatBarView, PREVIEW_LINE } from '../chat/DropsChatBar';
import EmoteAutocomplete from '../chat/EmoteAutocomplete';
import EmoteSearchList from '../chat/EmoteSearchList';
import { SpellcheckUnderlay } from '../chat/SpellcheckUnderlay';
import { useSpellcheck } from '../../hooks/useSpellcheck';
import { matchEmotes, rowsToTabCandidates, type EmoteMatchRow } from '../../services/emoteMatch';
import type { RoomModeView } from '../../stores/chatConnectionStore';

/** A channel's points as the wallet shows them (a made-up balance). */
const SAMPLE_POINTS: WalletPoints = { balance: 128_450, name: null, iconUrl: null, loading: false, lastGain: null };

/** What the box holds in each section: text with a misspelling for spellcheck,
 *  a half-typed emote name while Tab is shown, else nothing (the prompt). */
const TYPED: Record<string, string> = {
  input: 'is this the fianl boss',
  tab: 'that was so Ka',
};

export function PreviewComposer({
  section,
  roomModes,
  channel,
  roomId,
  emotesReady,
}: {
  /** `input`, `tab`, `drops`, or anything else. */
  section: string;
  roomModes: RoomModeView | null;
  channel: string;
  roomId: string;
  /** Bumps when the chat's emotes have loaded, so Tab asks again. */
  emotesReady: number;
}) {
  const prefs = useAppStore((s) => s.settings.chat_input);
  const dropsOn = useAppStore((s) => s.settings.chat_design?.show_drops_in_chat ?? true);
  const look = composerLook(prefs, {
    canSend: true,
    provider: 'twitch',
    signInOwed: false,
    watchStreak: false,
    resub: false,
    subOnlyBlocked: false,
    inputDisabled: false,
    roomModes,
  });
  const typed = TYPED[section] ?? '';
  const paddingLeft = composerPaddingLeft(false, look.showEmoteButton);
  const field = composerInputLook({ remindOverlay: false, watchStreak: false, commandValid: false, isCommand: false, paddingLeft });
  const spellRanges = useSpellcheck(typed, { enabled: prefs?.spellcheck_enabled ?? true, emoteKey: null });
  const walletButton = useRef<HTMLButtonElement>(null);

  // Tab completion: what Rust's matcher gives this chat for the half-typed word,
  // shown as your Tab style shows it.
  const tabOn = prefs?.emote_tab_complete_enabled ?? true;
  const tabStyle = prefs?.emote_tab_style === 'list' ? 'list' : 'carousel';
  const [tabRows, setTabRows] = useState<EmoteMatchRow[]>([]);
  useEffect(() => {
    if (section !== 'tab' || !tabOn) return;
    let live = true;
    const word = typed.split(' ').pop() ?? '';
    void matchEmotes({ provider: 'twitch', channel, channelId: roomId || null }, word, tabStyle === 'list' ? 'search' : 'cycle').then(
      (res) => {
        if (live) setTabRows(res.rows);
      },
    );
    return () => {
      live = false;
    };
  }, [section, tabOn, tabStyle, channel, roomId, typed, emotesReady]);
  const candidates = section === 'tab' && tabOn ? rowsToTabCandidates(tabRows) : [];
  const shown =
    tabStyle === 'carousel' && candidates.length > 0 ? `${typed.slice(0, typed.lastIndexOf(' ') + 1)}${candidates[0].name}` : typed;

  return (
    <>
      {section === 'drops' && dropsOn && (
        <div className="px-2">
          <DropsChatBarView line={PREVIEW_LINE} open={false} />
        </div>
      )}
      {/* Chat's input container (ChatWidget). */}
      <div
        className="flex-shrink-0 border-t border-borderSubtle"
        style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 94%, transparent)' }}
      >
        <div className="p-2">
          <div className="relative">
            <div className="sn-composer-row flex items-center gap-2 min-w-0">
              {look.showPointsBalance && (
                <WalletFace
                  points={SAMPLE_POINTS}
                  bits={null}
                  mode={look.pointsBalanceInline ? 'always' : 'hover'}
                  gains={look.walletGains}
                  open={false}
                  onToggle={() => {}}
                  chest={null}
                  buttonRef={walletButton}
                />
              )}
              <div className="sn-composer-field relative flex-1 min-w-0 flex items-center">
                {look.showEmoteButton && (
                  <EmoteSmileyButton open={false} smiley="😀" transitioning={false} label="Emotes, GIFs and commands" />
                )}
                <AnimatePresence>
                  {candidates.length > 0 && tabStyle === 'carousel' && (
                    <EmoteAutocomplete current={candidates[0]} backwards={[]} forwards={candidates.slice(1, 4)} />
                  )}
                  {tabRows.length > 0 && tabStyle === 'list' && section === 'tab' && tabOn && (
                    <EmoteSearchList
                      id="preview-emote-list"
                      query={typed.split(' ').pop() ?? ''}
                      rows={tabRows.slice(0, 8)}
                      total={tabRows.length}
                      ready
                      selectedIndex={0}
                      onSelect={() => {}}
                      onSelectedIndexChange={() => {}}
                    />
                  )}
                </AnimatePresence>
                <textarea
                  readOnly
                  tabIndex={-1}
                  value={shown}
                  rows={1}
                  spellCheck={false}
                  placeholder={look.placeholder}
                  {...field}
                />
                {look.roomModesChip && <RoomModeIndicator view={look.roomModesChip} />}
                {spellRanges.length > 0 && (
                  <SpellcheckUnderlay
                    text={shown}
                    ranges={spellRanges}
                    className="text-sm leading-[1.4]"
                    style={{ paddingTop: '8px', paddingBottom: '8px', paddingLeft, paddingRight: 'var(--sn-room-pad, 12px)' }}
                  />
                )}
              </div>
              <button type="button" tabIndex={-1} className={sendButtonClass(false)} disabled={!shown.trim()}>
                <PaperPlaneTilt size={20} weight="bold" aria-hidden />
              </button>
            </div>
          </div>
        </div>
      </div>
    </>
  );
}
