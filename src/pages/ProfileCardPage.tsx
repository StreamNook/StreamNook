import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { ProviderId } from '../types/providers';
import UserProfileCard from '../components/UserProfileCard';

const ProfileCardPage = () => {
  // Parse URL parameters manually
  const params = new URLSearchParams(window.location.hash.split('?')[1]);
  const userId = params.get('userId') || '';
  const username = params.get('username') || '';
  const displayName = params.get('displayName') || '';
  const color = params.get('color') || '#9146FF';
  const badgesStr = params.get('badges') || '[]';
  const badges = JSON.parse(badgesStr);
  const channelId = params.get('channelId') || '';
  const channelName = params.get('channelName') || '';
  // When opened from a context where the viewer can moderate this channel (e.g. the
  // MultiChat blended feed), surface the card's Moderator Actions. The broadcaster
  // target for those actions is this channel's id (channelId IS the broadcaster id).
  const isModerator = params.get('isModerator') === '1';
  const messageHistoryStr = params.get('messageHistory') || '[]';
  const messageHistory = JSON.parse(messageHistoryStr);
  // Which platform this chatter is on. Absent defaults to Twitch, so a popout URL
  // from before this existed still resolves correctly.
  const provider = (params.get('provider') || 'twitch') as ProviderId;

  // Enable window dragging
  useEffect(() => {
    const appWindow = getCurrentWindow();
    
    // Make the entire window draggable
    const handleMouseDown = async (e: MouseEvent) => {
      // Only start drag if clicking on the header area, but not on the close button
      const target = e.target as HTMLElement;
      const isHeader = target.closest('.profile-card-header');
      const isCloseButton = target.closest('button');
      
      if (isHeader && !isCloseButton) {
        e.preventDefault();
        await appWindow.startDragging();
      }
    };

    document.addEventListener('mousedown', handleMouseDown);

    return () => {
      document.removeEventListener('mousedown', handleMouseDown);
    };
  }, []);

  const handleClose = async () => {
    const appWindow = getCurrentWindow();
    await appWindow.close();
  };

  return (
    <div className="w-full h-full">
      <UserProfileCard
        userId={userId}
        username={username}
        displayName={displayName}
        color={color}
        badges={badges}
        messageHistory={messageHistory}
        onClose={handleClose}
        position={{ x: 0, y: 0 }} // Not used in window mode
        channelId={channelId}
        channelName={channelName}
        isModerator={isModerator}
        broadcasterId={channelId}
        provider={provider}
      />
    </div>
  );
};

export default ProfileCardPage;
