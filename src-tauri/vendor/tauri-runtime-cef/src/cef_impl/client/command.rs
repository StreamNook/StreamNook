// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Chrome's own commands inside an app window.
//!
//! Chrome style runs Chrome's accelerator table and menus inside every
//! browser: Ctrl+W closes the window, F5 and Ctrl+R reload the app, Ctrl+H,
//! Ctrl+J, Ctrl+U, Ctrl+T and Ctrl+P open Chrome's history, downloads,
//! view-source, new-tab and print windows. None of that belongs in an app, so
//! every Chrome command is handled here as a no-op except the clipboard and
//! editing commands a text field and its context menu need, and the DevTools
//! commands when DevTools are enabled for the webview. The same allowlist
//! prunes Chrome's context menu, so it never offers an item that would do
//! nothing.
//!
//! Commands are matched by their `IDC_` names through
//! `cef_id_for_command_id_name`: the numeric ids change between Chromium
//! versions, the names do not. A name the running CEF does not know resolves
//! to -1 and is skipped.

use std::{ffi::CStr, os::raw::c_int, sync::OnceLock};

use cef::*;

/// Clipboard and editing: the keyboard's cut/copy/paste family, Chrome's
/// text-field context menu, spelling replacements, and the copy items of the
/// link and image menus.
const EDITING_COMMANDS: &[&CStr] = &[
  c"IDC_CUT",
  c"IDC_COPY",
  c"IDC_PASTE",
  c"IDC_CONTENT_CONTEXT_CUT",
  c"IDC_CONTENT_CONTEXT_COPY",
  c"IDC_CONTENT_CONTEXT_PASTE",
  c"IDC_CONTENT_CONTEXT_PASTE_AND_MATCH_STYLE",
  c"IDC_CONTENT_CONTEXT_DELETE",
  c"IDC_CONTENT_CONTEXT_UNDO",
  c"IDC_CONTENT_CONTEXT_REDO",
  c"IDC_CONTENT_CONTEXT_SELECTALL",
  c"IDC_CONTENT_CONTEXT_COPYLINKLOCATION",
  c"IDC_CONTENT_CONTEXT_COPYLINKTEXT",
  c"IDC_CONTENT_CONTEXT_COPYIMAGE",
  c"IDC_CONTENT_CONTEXT_COPYIMAGELOCATION",
  c"IDC_CONTENT_CONTEXT_COPYAVLOCATION",
  c"IDC_SPELLCHECK_SUGGESTION_0",
  c"IDC_SPELLCHECK_SUGGESTION_1",
  c"IDC_SPELLCHECK_SUGGESTION_2",
  c"IDC_SPELLCHECK_SUGGESTION_3",
  c"IDC_SPELLCHECK_SUGGESTION_4",
];

/// DevTools: allowed only for a webview with DevTools enabled.
const DEVTOOLS_COMMANDS: &[&CStr] = &[
  c"IDC_DEV_TOOLS",
  c"IDC_DEV_TOOLS_CONSOLE",
  c"IDC_DEV_TOOLS_DEVICES",
  c"IDC_DEV_TOOLS_INSPECT",
  c"IDC_DEV_TOOLS_TOGGLE",
  c"IDC_CONTENT_CONTEXT_INSPECTELEMENT",
  c"IDC_CONTENT_CONTEXT_INSPECTBACKGROUNDPAGE",
];

fn resolve(names: &[&CStr]) -> Vec<c_int> {
  names
    .iter()
    // SAFETY: a NUL-terminated name; CEF only reads it.
    .map(|name| unsafe { cef::sys::cef_id_for_command_id_name(name.as_ptr()) })
    .filter(|id| *id >= 0)
    .collect()
}

fn editing_ids() -> &'static [c_int] {
  static IDS: OnceLock<Vec<c_int>> = OnceLock::new();
  IDS.get_or_init(|| resolve(EDITING_COMMANDS))
}

fn devtools_ids() -> &'static [c_int] {
  static IDS: OnceLock<Vec<c_int>> = OnceLock::new();
  IDS.get_or_init(|| resolve(DEVTOOLS_COMMANDS))
}

/// Whether Chrome may run `command_id` in a webview.
pub(crate) fn command_allowed(command_id: c_int, devtools_enabled: bool) -> bool {
  editing_ids().contains(&command_id) || (devtools_enabled && devtools_ids().contains(&command_id))
}

/// Keep only the items of Chrome's context menu that would do something:
/// allowed commands, and the separators between them.
pub(crate) fn prune_context_menu(model: &mut MenuModel, devtools_enabled: bool) {
  for index in (0..model.count()).rev() {
    let separator = model.type_at(index) == MenuItemType::SEPARATOR;
    if !separator && !command_allowed(model.command_id_at(index), devtools_enabled) {
      model.remove_at(index);
    }
  }
  // Separators that no longer separate anything: leading, trailing, doubled.
  let mut previous_was_separator = true;
  let mut index = 0;
  while index < model.count() {
    let separator = model.type_at(index) == MenuItemType::SEPARATOR;
    if separator && previous_was_separator {
      model.remove_at(index);
      continue;
    }
    previous_was_separator = separator;
    index += 1;
  }
  if let Some(last) = model.count().checked_sub(1)
    && model.type_at(last) == MenuItemType::SEPARATOR
  {
    model.remove_at(last);
  }
}

wrap_command_handler! {
  pub struct TauriCefCommandHandler {
    devtools_enabled: bool,
  }

  impl CommandHandler {
    fn on_chrome_command(
      &self,
      _browser: Option<&mut Browser>,
      command_id: c_int,
      _disposition: WindowOpenDisposition,
    ) -> c_int {
      // 1 = handled (nothing happens), 0 = Chrome's default.
      c_int::from(!command_allowed(command_id, self.devtools_enabled))
    }

    fn is_chrome_app_menu_item_visible(
      &self,
      _browser: Option<&mut Browser>,
      command_id: c_int,
    ) -> c_int {
      c_int::from(command_allowed(command_id, self.devtools_enabled))
    }

    fn is_chrome_page_action_icon_visible(&self, _icon_type: ChromePageActionIconType) -> c_int {
      0
    }

    fn is_chrome_toolbar_button_visible(&self, _button_type: ChromeToolbarButtonType) -> c_int {
      0
    }
  }
}
