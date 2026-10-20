// The You tab: your identity card as the screen's header (no title above it:
// the card says whose tab this is, and its Edit profile row is the way in to
// every picture, name and cosmetic), the accounts you are signed in with (each
// with its own sign in / sign out), then settings sections inline.
import React, { useEffect, useState } from 'react';
import { ArrowCircleDown, At, ChatCircle } from 'phosphor-react';
import { useMentionsStore } from '../mentions/mentionInbox';
import { useWhispersStore } from '../whispers/whisperInbox';
import { WHISPER_WORDS } from '../whispers/whisperModel';
import { ChevronRight } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore, type MobileTab } from '../navStore';
import { usePhonePrefs } from '../phonePrefs';
import { SegmentedControl, type Segment } from '../ui/SegmentedControl';
import { checkForAndroidUpdate, openAndroidUpdate, type AndroidUpdate } from '../updateCheck';
import { OwnIdentityCard, SignedOutIdentityCard } from '../profile/OwnIdentityHeader';
import { SettingsDirectory } from './SettingsSearch';
import { AccountsCard } from '../profile/AccountsCard';

const START_TABS: Segment<MobileTab>[] = [
  { value: 'following', label: 'Following' },
  { value: 'browse', label: 'Browse' },
  { value: 'chats', label: 'Chats' },
  { value: 'rewards', label: 'Rewards' },
  { value: 'you', label: 'You' },
];

export const YouScreen: React.FC = () => {
  const currentUser = useAppStore((s) => s.currentUser);
  const isAuthenticated = useAppStore((s) => s.isAuthenticated);
  const setCosmeticsOpen = useMobileNavStore((s) => s.setCosmeticsOpen);
  const setMentionsOpen = useMobileNavStore((s) => s.setMentionsOpen);
  const mentionsUnread = useMentionsStore((s) => s.unreadTotal);
  const openWhispers = useMobileNavStore((s) => s.openWhispers);
  const whispersUnread = useWhispersStore((s) => s.unreadTotal);
  const startTab = usePhonePrefs((s) => s.startTab);
  const setStartTab = usePhonePrefs((s) => s.setStartTab);
  const addToast = useAppStore((s) => s.addToast);

  // Checked once when this tab mounts rather than at boot: a sideloaded app has
  // no store to notify anyone, but an update prompt is also not worth delaying
  // startup or interrupting a stream for. You is where app-level things live.
  const [update, setUpdate] = useState<AndroidUpdate | null>(null);
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const found = await checkForAndroidUpdate();
      if (!cancelled) setUpdate(found);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="sn-mobile-screen sn-tabbar-clearance">
      <div className="px-4 pt-2 pb-5">
        {currentUser?.user_id ? (
          <OwnIdentityCard
            userId={currentUser.user_id}
            displayName={currentUser.display_name || currentUser.username || currentUser.login || ''}
            login={currentUser.login}
            platformUrl={currentUser.profile_image_url}
            onEdit={() => setCosmeticsOpen(true)}
          />
        ) : (
          <SignedOutIdentityCard waiting={isAuthenticated} />
        )}
      </div>

      {/* Only rendered when there is genuinely a newer build. Sideloaded apps
          get no store notification, so without this nobody ever learns an
          update exists. Tapping opens the public download page and Android's
          package installer takes it from there. */}
      {update && (
        <div className="px-3 mb-2">
          <button
            onClick={() =>
              void openAndroidUpdate().then((ok) => {
                if (!ok) addToast('Could not open the download page.', 'error');
              })
            }
            className="w-full flex items-center gap-3 px-3 py-3 rounded-lg bg-accent/10 active:bg-accent/20 text-left"
          >
            <ArrowCircleDown size={20} weight="fill" className="text-accent shrink-0" />
            <span className="flex-1 min-w-0">
              <span className="block text-[15px] font-semibold text-textPrimary">
                Update to {update.latest}
              </span>
              <span className="block text-[12.5px] text-textMuted truncate">
                You are on {update.current}
                {update.size ? ` · ${(update.size / 1048576).toFixed(0)} MB` : ''}
              </span>
            </span>
            <ChevronRight size={16} className="text-accent shrink-0" />
          </button>
        </div>
      )}

      {/* Your mentions, first among the things that are about you, then your
          whispers (Twitch's, so only with a Twitch sign-in; here until the chat
          inbox opens them). The dot is the only mark unread gets here. */}
      <div className="px-4 mb-4">
        <div className="glass-panel w-full overflow-hidden">
          <button
            onClick={() => setMentionsOpen(true)}
            className="w-full flex items-center gap-3 px-3.5 py-3 text-left active:bg-surface-active"
          >
            <At size={20} className="text-textSecondary shrink-0" />
            <span className="flex-1 min-w-0">
              <span className="block text-[15px] font-medium text-textPrimary">Mentions</span>
              <span className="block text-[12.5px] text-textMuted truncate">
                Who mentioned or replied to you in chat
              </span>
            </span>
            {mentionsUnread > 0 && (
              <span className="w-2 h-2 rounded-full bg-accent shrink-0" aria-label="Unread mentions" />
            )}
            <ChevronRight size={16} className="text-textMuted shrink-0" />
          </button>
          {currentUser?.user_id && (
            <>
              <div className="ml-[46px] border-t border-borderSubtle" />
              <button
                onClick={openWhispers}
                className="w-full flex items-center gap-3 px-3.5 py-3 text-left active:bg-surface-active"
              >
                <ChatCircle size={20} className="text-textSecondary shrink-0" />
                <span className="flex-1 min-w-0">
                  <span className="block text-[15px] font-medium text-textPrimary">{WHISPER_WORDS.title}</span>
                  <span className="block text-[12.5px] text-textMuted truncate">{WHISPER_WORDS.entryDetail}</span>
                </span>
                {whispersUnread > 0 && (
                  <span className="w-2 h-2 rounded-full bg-accent shrink-0" aria-label="Unread whispers" />
                )}
                <ChevronRight size={16} className="text-textMuted shrink-0" />
              </button>
            </>
          )}
        </div>
      </div>

      {/* Sign-in and sign-out live on each account's own row, so the action
          reads as acting on the account beside it, and nothing about signing
          out is hidden behind the floating tab bar. */}
      <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide px-4 mb-1">
        Accounts
      </div>
      <div className="px-4 mb-4">
        <AccountsCard />
      </div>

      {/* Where the app lands on open. Following is the right default for most
          people, but someone who lives in Browse or checks drops first should
          not have to swipe past it every time; back unwinds to this tab too. */}
      <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide px-4 mb-1">
        Open the app on
      </div>
      {/* The segments share the row's width, so the track ends where the
          cards above and below it end; each is sized to its label, so all five
          fit on one line. */}
      <div className="px-4 mb-4">
        <SegmentedControl<MobileTab>
          label="Open the app on"
          value={startTab}
          onChange={setStartTab}
          segments={START_TABS}
          fill
        />
      </div>

      <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide px-4 mb-1">
        Settings
      </div>
      {/* The search field over the section rows; while something is typed
          the rows give way to the settings that match. */}
      <SettingsDirectory />
    </div>
  );
};
