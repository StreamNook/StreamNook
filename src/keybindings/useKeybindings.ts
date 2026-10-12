// Global keybinding dispatcher.
//
// Mounted once per window. A single capture-phase keydown listener resolves the
// active context (global, and player when a stream is mounted), matches the
// pressed chord against effective bindings, and fires the first runnable match.
// A `deferred` match (a key the page answers too, like Esc) is left alone in
// that pass and run at the very end of the press instead, once the page has
// had the key, and only if nothing on the way marked it handled.
//
// Safety rules that make single-key binds (M, F, Space) usable in a chat app:
//   - stands down while the settings recorder is capturing,
//   - stands down while the command palette or settings dialog is open (they
//     own their own keys),
//   - never fires a bind without a Ctrl/Alt/Meta modifier while the user is
//     typing in an input/textarea/contentEditable.

import { useEffect } from 'react';
import { useAppStore } from '../stores/AppStore';
import { getBindableCommands } from './commands';
import { getEffectiveBindings, getOverrides } from './registry';
import { eventToCanonical, isModifierEvent } from './chord';
import { isPlayerControllable } from './playerControls';
import { getChatModController } from './chatModController';
import { isRecordingKeybind } from './recorderState';
import type { BindableCommand, KeybindContext } from './types';

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  // isContentEditable is true for any node inside an editable region, so this
  // covers nested rich-text editors. closest() catches the rare case where the
  // event target is a wrapper around an input. Mirrors App.tsx's input guard.
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
  return target.closest('input, textarea, select, [contenteditable=""], [contenteditable="true"]') !== null;
}

function activeContexts(): Set<KeybindContext> {
  const s = useAppStore.getState();
  const set = new Set<KeybindContext>(['global']);
  if (s.currentStream && isPlayerControllable()) set.add('player');
  // chatPane carries moderation keys; only live for moderators/broadcasters.
  if (getChatModController()?.isModerator()) set.add('chatPane');
  return set;
}

// Player-context binds beat global when both could match, so single keys like
// Space reach the player rather than a global action. When a chat message is
// focused for moderation, the chat context jumps ahead of the player so single
// keys (K, B, digits, ...) drive moderation; otherwise the player keeps them
// (e.g. K = play/pause), and J is the non-conflicting key that starts focusing.
function contextPriority(): KeybindContext[] {
  if (getChatModController()?.hasFocus()) return ['chatPane', 'player', 'global'];
  return ['player', 'chatPane', 'global'];
}

type KeyTarget = Pick<Window, 'addEventListener'>;

/**
 * Runs a deferred match once the page has had the key. The listener is added
 * now, during the capture pass, so it is the last of the window's bubble
 * listeners to see this press: every handler on the way, the window's own
 * included, has had its chance to answer it. One that called preventDefault
 * answered it; one that called stopPropagation kept it from ever arriving, and
 * the listener then clears itself on the next press.
 */
function runAfterPage(target: KeyTarget, e: KeyboardEvent, cmd: BindableCommand): void {
  const late = (ev: Event) => {
    if (ev !== e || e.defaultPrevented) return;
    // Asked again: a handler on the way may have changed what is on screen.
    if (cmd.isAvailable && !cmd.isAvailable()) return;
    e.preventDefault();
    if (e.repeat && !cmd.repeatable) return;
    try {
      void cmd.run?.();
    } catch {
      // Actions surface their own error toasts.
    }
  };
  target.addEventListener('keydown', late, { once: true });
}

/** The capture-phase keydown handler, built once per window. `target` is the
 *  window it listens on, where deferred matches wait for the end of the press. */
export function createKeybindingDispatcher(target: KeyTarget): (e: KeyboardEvent) => void {
  const onKeyDown = (e: KeyboardEvent) => {
    if (isRecordingKeybind()) return;
    // Ignore IME composition keystrokes (CJK input, dead keys): the
    // composition owns the keyboard until it commits.
    if (e.isComposing || e.keyCode === 229) return;
    if (isModifierEvent(e)) return;
    // AltGr (Ctrl+Alt on Windows) is used to type characters on many EU
    // layouts. Treat it as text entry, never a shortcut, so binds like
    // Ctrl+Alt+R do not fire while composing an AltGr glyph.
    if (typeof e.getModifierState === 'function' && e.getModifierState('AltGraph')) return;

    const state = useAppStore.getState();
    // Stand down while a surface that owns the keyboard is open: the command
    // palette, the settings dialog, or a full-window overlay (its own buttons
    // / Esc handlers / search inputs should win, and player single-keys must
    // not drive the hidden player underneath).
    if (
      state.isCommandPaletteOpen ||
      state.isSettingsOpen ||
      state.showDropsOverlay ||
      state.showBadgesOverlay ||
      state.showWhispersOverlay ||
      state.showDashboardOverlay ||
      state.profileModalUser
    ) {
      return;
    }

    // Typing guard: while focus is in any text field, only Ctrl/Cmd chords are
    // allowed to fire. Bare keys, Shift+key, and Alt+key are all treated as
    // text (Alt composes characters on some layouts), so nothing you type into
    // chat can ever trigger a binding — not a letter, not a digit, not a
    // symbol. Ctrl/Cmd combos (e.g. Ctrl+K) never produce text, so they stay.
    if (isEditableTarget(e.target) && !(e.ctrlKey || e.metaKey)) return;

    const canonical = eventToCanonical(e);
    const contexts = activeContexts();
    const overrides = getOverrides();
    const commands = getBindableCommands();

    for (const ctx of contextPriority()) {
      if (!contexts.has(ctx)) continue;
      for (const cmd of commands) {
        if (cmd.reserved || !cmd.run || cmd.context !== ctx) continue;
        if (cmd.isAvailable && !cmd.isAvailable()) continue;
        if (!getEffectiveBindings(cmd, overrides).includes(canonical)) continue;
        // Matched, but the page answers this key too: leave it alone and
        // run at the end of the press if nothing claims it first.
        if (cmd.deferred) {
          runAfterPage(target, e, cmd);
          return;
        }
        // Matched: always swallow the key so the native default never leaks
        // through (e.g. Space scrolling). But suppress OS key auto-repeat for
        // non-repeatable actions so a held toggle fires exactly once.
        e.preventDefault();
        e.stopPropagation();
        if (e.repeat && !cmd.repeatable) return;
        try {
          void cmd.run();
        } catch {
          // Actions surface their own error toasts.
        }
        return;
      }
    }
  };

  return onKeyDown;
}

export function useKeybindings(): void {
  useEffect(() => {
    const onKeyDown = createKeybindingDispatcher(window);
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, []);
}
