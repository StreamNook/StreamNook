// Opening a link outside the app, on either shell.
//
// This lives in utils/ rather than mobile/ ON PURPOSE. Shared chat components
// (ChatMessage, LinkPreviewCard) need it, and nothing under src/components,
// src/utils or src/services imports from src/mobile - keeping that boundary
// clean is what stops the mobile chunk being dragged into the desktop bundle.
//
// DESKTOP keeps `@tauri-apps/plugin-shell`'s `open`, unchanged.
//
// ANDROID cannot use it, and that is the whole reason this helper exists. The
// shell plugin's `open` command is not cfg-split for mobile: it bottoms out in
// the `open` crate's `that_detached`, which launches a helper program - xdg-open,
// gio open, gnome-open. Android has none of them, so the invoke REJECTS. Every
// chat link tap was hitting that, and the rejection was swallowed by a catch
// that only logged, which on a release build (no console, no devtools) is
// indistinguishable from an element that does nothing.
//
// The shell plugin does ship a working ShellPlugin.kt with a real ACTION_VIEW,
// but it is only reachable from a Rust-only `#[cfg(mobile)] pub fn open` that
// nothing calls - so "add the Kotlin implementation" is not the fix.
//
// `tauri-plugin-opener` is the supported replacement (shell's open is marked
// deprecated in favour of it) and its OpenerPlugin.kt fires a plain
// Intent(ACTION_VIEW, url) with FLAG_ACTIVITY_NEW_TASK. That is exactly the
// Android App Links path, which is what lets a discord.gg link open in the
// Discord app rather than a browser tab.
//
// Invoked by command name rather than through `@tauri-apps/plugin-opener`, to
// avoid a JS dependency for a single call.
//
// The grant needs a SCOPE, which is the part that is easy to miss.
// `opener:allow-open-url` on its own enables the command with an EMPTY
// allow-list, and `open_url` checks every url against that list before doing
// anything - so the bare permission turns "cannot open" into "forbidden url",
// which looks identical from the outside. capabilities/mobile.json grants it as
// an object with `allow: [{ url: "https://*" }, { url: "http://*" }]`.
import { invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';
import { IS_MOBILE } from './platform';

/**
 * Hands a url to the OS. Returns whether it was accepted.
 *
 * Callers are expected to tell the user when this returns false. Logging and
 * returning is what made the original bug invisible.
 */
export async function openExternal(url: string): Promise<boolean> {
  try {
    if (IS_MOBILE) {
      await invoke('plugin:opener|open_url', { url });
    } else {
      const { open } = await import('@tauri-apps/plugin-shell');
      await open(url);
    }
    return true;
  } catch (err) {
    Logger.error('[openExternal] could not open', url, err);
    return false;
  }
}
