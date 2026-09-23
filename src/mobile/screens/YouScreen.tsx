// The You tab: who you are, the accounts you are signed in with (each with its
// own sign in / sign out), then settings sections inline (no intermediate menu).
import React, { useEffect, useState } from 'react';
import { ArrowCircleDown, PaintBrush } from 'phosphor-react';
import { ChevronRight } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore, type MobileTab } from '../navStore';
import { usePhonePrefs } from '../phonePrefs';
import { SegmentedSelect } from '../../components/settings/_primitives';
import { checkForAndroidUpdate, openAndroidUpdate, type AndroidUpdate } from '../updateCheck';
import { OwnIdentityHeader } from '../profile/OwnIdentityHeader';
import { SETTINGS_ROWS } from './SettingsScreen';
import { AccountsCard } from '../profile/AccountsCard';

export const YouScreen: React.FC = () => {
  const currentUser = useAppStore((s) => s.currentUser);
  const openSettings = useMobileNavStore((s) => s.openSettings);
  const setCosmeticsOpen = useMobileNavStore((s) => s.setCosmeticsOpen);
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
      {/* items-start, not items-center: the identity block can run to two lines
          for a long name plus a badge row, and centring against it would drag
          the avatar off the name it belongs to. */}
      <div className="flex items-start gap-3 px-4 pt-4 pb-3">
        {currentUser?.profile_image_url ? (
          <img
            src={currentUser.profile_image_url}
            alt=""
            className="w-14 h-14 rounded-full shrink-0"
            draggable={false}
          />
        ) : (
          <div className="w-14 h-14 rounded-full bg-surface shrink-0" />
        )}
        {currentUser?.user_id ? (
          <OwnIdentityHeader
            userId={currentUser.user_id}
            displayName={currentUser.display_name || currentUser.username || 'Signed in'}
            login={currentUser.login}
          />
        ) : (
          <div className="min-w-0 flex-1">
            <div className="text-lg font-bold text-textPrimary truncate">Signed in</div>
          </div>
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

      <div className="px-3 mb-2">
        <button
          onClick={() => setCosmeticsOpen(true)}
          className="w-full flex items-center gap-3 px-3 py-3 rounded-lg active:bg-surface-active text-left"
        >
          <PaintBrush size={20} className="text-accent shrink-0" />
          <span className="flex-1 min-w-0">
            <span className="block text-[15px] font-medium text-textPrimary">Cosmetics</span>
            <span className="block text-[12.5px] text-textMuted truncate">
              Your badges, atmosphere, and 7TV paint
            </span>
          </span>
          <ChevronRight size={16} className="text-textMuted shrink-0" />
        </button>
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
      <div className="px-4 mb-4">
        <SegmentedSelect<MobileTab>
          value={startTab}
          onChange={setStartTab}
          fullWidth
          options={[
            { value: 'following', label: 'Following' },
            { value: 'browse', label: 'Browse' },
            { value: 'rewards', label: 'Rewards' },
            { value: 'you', label: 'You' },
          ]}
        />
      </div>

      <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide px-4 mb-1">
        Settings
      </div>
      <div className="px-3">
        {SETTINGS_ROWS.map(({ id, label, icon: Icon, description }) => (
          <button
            key={id}
            onClick={() => openSettings(id)}
            className="w-full flex items-center gap-3 px-3 py-3 rounded-lg active:bg-surface-active text-left"
          >
            <Icon size={20} className="text-textSecondary shrink-0" />
            <span className="flex-1 min-w-0">
              <span className="block text-[15px] font-medium text-textPrimary">{label}</span>
              <span className="block text-[12.5px] text-textMuted truncate">{description}</span>
            </span>
            <ChevronRight size={16} className="text-textMuted shrink-0" />
          </button>
        ))}
      </div>
    </div>
  );
};
