// "Add a chat": the chat picker in a sheet, from the Chats strip's + and the
// chat menu. The lists and search live in ChatPicker.
import React from 'react';
import { MobileSheet } from '../ui/MobileSheet';
import { ChatPicker } from './ChatPicker';

export const AddChatSheet: React.FC<{ open: boolean; onClose: () => void }> = ({ open, onClose }) => (
  <MobileSheet open={open} onClose={onClose} title="Add a chat" maxHeightFraction={0.85} fill>
    <ChatPicker onPicked={onClose} />
  </MobileSheet>
);
