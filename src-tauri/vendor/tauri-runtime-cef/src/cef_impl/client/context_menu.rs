// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use cef::*;

wrap_context_menu_handler! {
  pub struct TauriCefContextMenuHandler {
    devtools_enabled: bool,
  }

  impl ContextMenuHandler {
    fn on_before_context_menu(
      &self,
      _browser: Option<&mut Browser>,
      _frame: Option<&mut Frame>,
      _params: Option<&mut ContextMenuParams>,
      model: Option<&mut MenuModel>,
    ) {
      // Under Chrome style the command handler refuses every command outside
      // its allowlist, so the menu keeps only those items (Inspect included,
      // when DevTools are on).
      #[cfg(not(target_os = "macos"))]
      if let Some(model) = model {
        super::command::prune_context_menu(model, self.devtools_enabled);
      }
      #[cfg(target_os = "macos")]
      if !self.devtools_enabled
        && let Some(model) = model
      {
        model.remove_at(model.count() - 1);
      }
    }
  }
}
