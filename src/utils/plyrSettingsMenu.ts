// The player's gear menu is Plyr's own markup, so this glue stays in TS: it
// only arranges a third-party control's elements, with no app state to own.
//
// Plyr builds the menu as
//   .plyr__menu__container
//     div                      (the panes' frame; Plyr sizes it while it animates)
//       div#…-home             (the main list: Speed, and our injected Quality row)
//       div#…-speed [hidden]   (Plyr's Speed list)
//     div[data-quality-menu]   (our Quality list, appended by VideoPlayer)
// and remembers whichever pane was open when the menu closed.

/**
 * Readies the gear menu as it opens (SN-277): it wears the glaze flyout like
 * every other menu, and it always opens on its main list, never on the Quality
 * or Speed list it was closed on.
 */
export function prepareSettingsMenu(menu: Element): void {
  menu.classList.add('glass-flyout');
  const home = menu.querySelector('[id$="-home"]');
  if (!home) return;
  const frame = home.parentElement;
  if (frame) {
    for (const pane of Array.from(frame.children)) {
      if (pane === home) pane.removeAttribute('hidden');
      else pane.setAttribute('hidden', '');
    }
    // A menu closed mid-animation keeps the size Plyr set for the pane it was
    // moving to; the main list measures itself again.
    if (frame instanceof HTMLElement) {
      frame.style.width = '';
      frame.style.height = '';
    }
  }
  // Opening our Quality list hides the main list's own menu element.
  for (const list of Array.from(home.querySelectorAll('[role="menu"][hidden]'))) list.removeAttribute('hidden');
  for (const quality of Array.from(menu.querySelectorAll('[data-quality-menu]'))) quality.setAttribute('hidden', '');
}
