/**
 * Visibility policy for the player-overlay action buttons
 * (settings.player_overlay_buttons). Undefined = show all; otherwise a button
 * shows only when its id is present. Shared by every surface that renders
 * follow/subscribe-style channel actions so the setting is honored everywhere.
 */
export function playerOverlayButtonOn(buttons: string[] | undefined, id: string): boolean {
  return !buttons || buttons.includes(id);
}
