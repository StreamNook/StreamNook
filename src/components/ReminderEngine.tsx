import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { fireReminderFromRust } from '../utils/reminderEngine';

// Headless controller for reminders. Rust decides when one fires (timed triggers
// and chat keywords, services/reminder_service.rs) and announces it; this posts
// it from the main window, whose chat store holds the sent row. Never re-renders.
const ReminderEngine = () => {
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    void listen<{ id: string; channel: string; is_current: boolean }>('reminders://fire', (event) => {
      const { id, channel, is_current } = event.payload;
      void fireReminderFromRust(id, channel, is_current);
    }).then((fn) => {
      if (active) unlisten = fn;
      else fn();
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);
  return null;
};

export default ReminderEngine;
