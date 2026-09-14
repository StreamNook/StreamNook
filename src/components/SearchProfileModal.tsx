import React, { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { User, UserPlus, UserMinus, Loader2, ExternalLink, X } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';

import { useAppStore } from '../stores/AppStore';
import { TwitchStream } from '../types';
import StreamerAboutPanel from './StreamerAboutPanel';
import StreamerMedia from './StreamerMedia';
import { Tooltip } from './ui/Tooltip';
import { Logger } from '../utils/logger';

type ProfileTab = 'about' | 'clips' | 'videos';

export const SearchProfileModal = ({ user, onClose }: { user: TwitchStream, onClose: () => void }) => {
    const [isFollowing, setIsFollowing] = useState<boolean | null>(null);
    const [followLoading, setFollowLoading] = useState(false);
    // The header avatar must show the profile picture, not the stream preview.
    const [profileImg, setProfileImg] = useState<string | null>(user.profile_image_url || null);
    // Opens on About from a name-click; on Clips from the player-overlay button.
    const [tab, setTab] = useState<ProfileTab>(() => useAppStore.getState().profileModalInitialTab);

    useEffect(() => {
        let mounted = true;
        const isLiveFollowed = useAppStore.getState().followedStreams.some(s => s.user_id === user.user_id);
        
        if (isLiveFollowed) {
            setIsFollowing(true);
        } else {
            const checkBackend = async () => {
                try {
                    const status = await invoke<boolean>('check_following_status', { targetUserId: user.user_id });
                    if (mounted) setIsFollowing(status);
                } catch (e) {
                    Logger.warn('[SearchProfileModal] Failed to check follow status:', e);
                    if (mounted) setIsFollowing(false);
                }
            };
            checkBackend();
        }

        return () => { mounted = false; };
    }, [user.user_id]);

    // Helix stream objects (e.g. the currently-watched stream) carry only
    // thumbnail_url (the live preview), not a profile image, so fetch the real
    // one when it's missing. Never falls back to the stream thumbnail.
    useEffect(() => {
        if (user.profile_image_url) { setProfileImg(user.profile_image_url); return; }
        let mounted = true;
        invoke<{ profile_image_url?: string | null }>('get_channel_about_data', { channelLogin: user.user_login })
            .then((data) => { if (mounted && data?.profile_image_url) setProfileImg(data.profile_image_url); })
            .catch((e) => Logger.warn('[SearchProfileModal] profile image fetch failed:', e));
        return () => { mounted = false; };
    }, [user.user_login, user.profile_image_url]);

    const handleFollowAction = async () => {
        setFollowLoading(true);
        try {
            const command = isFollowing ? 'unfollow_channel' : 'follow_channel';
            await invoke(command, { targetUserId: user.user_id });
            setIsFollowing(!isFollowing);
            useAppStore.getState().addToast(`Successfully ${isFollowing ? 'unfollowed' : 'followed'} ${user.user_name}`, 'success');
        } catch (err) {
            Logger.error('Failed to toggle follow status:', err);
            useAppStore.getState().addToast(`Failed to ${isFollowing ? 'unfollow' : 'follow'} ${user.user_name}`, 'error');
        } finally {
            setFollowLoading(false);
        }
    };

    const isLive = user.is_live === true || (user as any).type === 'live';

    return (
        <AnimatePresence>
            <motion.div
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.2 }}
                className="fixed inset-0 z-[100] bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
                onClick={onClose}
            >
                <motion.div
                    initial={{ opacity: 0, scale: 0.95, y: 15 }}
                    animate={{ opacity: 1, scale: 1, y: 0 }}
                    exit={{ opacity: 0, scale: 0.95, y: 15 }}
                    transition={{ type: "spring", stiffness: 350, damping: 25 }}
                    className="w-full max-w-7xl h-[90vh] glass-panel backdrop-blur-xl border border-borderLight rounded-xl shadow-2xl flex flex-col overflow-hidden"
                    onClick={e => e.stopPropagation()}
                >
                    {/* Header */}
                    <div className="flex items-center justify-between p-4 border-b border-borderSubtle bg-glass/30">
                        <div className="flex items-center gap-3 w-1/2">
                            <div className={`relative w-12 h-12 rounded-full flex items-center justify-center overflow-hidden ring-2 ${isLive ? 'ring-red-500/80' : 'bg-accent/20 ring-accent/20'}`}>
                                {profileImg ? (
                                    <img src={profileImg} alt={user.user_name} className="w-full h-full object-cover" />
                                ) : (
                                    <User size={24} className={isLive ? 'text-red-500' : 'text-accent'} />
                                )}
                                {isLive && (
                                    <div className="absolute -bottom-1 left-1/2 -translate-x-1/2 live-dot text-[8px] px-1 py-0 shadow-lg">LIVE</div>
                                )}
                            </div>
                            <div className="flex-1 min-w-0">
                                <h2 className="text-xl font-bold text-textPrimary leading-tight truncate">{user.user_name}</h2>
                                {isLive && user.game_name ? (
                                    <Tooltip content={user.game_name}>
                                    <p className="text-sm text-accent truncate w-full">
                                        Playing {user.game_name} • {user.viewer_count.toLocaleString()} viewers
                                    </p>
                                    </Tooltip>
                                ) : (
                                    <p className="text-sm text-textSecondary">@{user.user_login}</p>
                                )}
                            </div>
                        </div>
                        <div className="flex items-center gap-3">
                            <button
                                onClick={handleFollowAction}
                                disabled={followLoading || isFollowing === null}
                                className={`glass-button text-sm py-2 px-4 rounded-lg flex items-center justify-center gap-2 min-w-[110px] ${followLoading || isFollowing === null
                                    ? 'opacity-50 cursor-wait'
                                    : isFollowing
                                        ? 'hover:bg-red-500/20 text-red-400 border-red-500/30'
                                        : 'hover:bg-green-500/20 text-green-400 border-green-500/30'
                                    }`}
                            >
                                {followLoading || isFollowing === null ? (
                                    <>
                                        <Loader2 size={16} className="animate-spin" />
                                        <span>Checking...</span>
                                    </>
                                ) : isFollowing ? (
                                    <>
                                        <UserMinus size={16} />
                                        <span>Unfollow</span>
                                    </>
                                ) : (
                                    <>
                                        <UserPlus size={16} />
                                        <span>Follow</span>
                                    </>
                                )}
                            </button>
                            <a href={`https://www.twitch.tv/${user.user_login}`} target="_blank" rel="noopener noreferrer" className="glass-button text-textSecondary hover:text-white text-sm py-2 px-4 rounded-lg flex items-center gap-2">
                                <ExternalLink size={16} />
                                Channel
                            </a>
                            <button onClick={onClose} className="p-2 text-textSecondary hover:text-white hover:bg-glass hover:text-red-400 rounded-lg transition-all ml-2 border border-transparent hover:border-red-500/30">
                                <X size={24} />
                            </button>
                        </div>
                    </div>

                    {/* Tabs */}
                    <div className="flex items-center gap-2 px-4 py-2.5 border-b border-borderSubtle bg-glass/20">
                        {(['about', 'clips', 'videos'] as const).map((t) => (
                            <button
                                key={t}
                                onClick={() => setTab(t)}
                                className={`px-4 py-1.5 text-sm font-bold !rounded-lg transition-all ${
                                    tab === t
                                        ? 'glass-input text-textPrimary'
                                        : 'glass-button text-textSecondary/80 hover:text-textPrimary'
                                }`}
                            >
                                {t === 'about' ? 'About' : t === 'clips' ? 'Clips' : 'Videos'}
                            </button>
                        ))}
                    </div>

                    {/* Body */}
                    <div className="flex-1 overflow-hidden relative bg-black/20">
                        {tab === 'about' && <StreamerAboutPanel channelLogin={user.user_login} hideHero={true} onChannelSwitch={onClose} />}
                        {tab === 'clips' && <StreamerMedia broadcasterId={user.user_id} kind="clips" />}
                        {tab === 'videos' && <StreamerMedia broadcasterId={user.user_id} kind="videos" />}
                    </div>
                </motion.div>
            </motion.div>
        </AnimatePresence>
    );
};
