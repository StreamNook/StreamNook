// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import { prepareSettingsMenu } from './plyrSettingsMenu';

// The menu as Plyr builds it, with VideoPlayer's Quality list appended, left
// the way zoox found it (SN-277): closed while the Quality list was showing.
function closedOnQuality(): HTMLElement {
  const menu = document.createElement('div');
  menu.className = 'plyr__menu__container';
  menu.innerHTML = `
    <div style="width: 150px; height: 300px">
      <div id="plyr-settings-1-home"><div role="menu" hidden><button data-plyr="quality"></button></div></div>
      <div id="plyr-settings-1-speed" hidden><div role="menu"></div></div>
    </div>
    <div role="menu" data-quality-menu></div>`;
  return menu;
}

describe('prepareSettingsMenu', () => {
  it('opens on the main list whichever list it was closed on', () => {
    const menu = closedOnQuality();
    prepareSettingsMenu(menu);
    const home = menu.querySelector('#plyr-settings-1-home')!;
    expect(home.hasAttribute('hidden')).toBe(false);
    expect(home.querySelector('[role="menu"]')!.hasAttribute('hidden')).toBe(false);
    expect(menu.querySelector('[data-quality-menu]')!.hasAttribute('hidden')).toBe(true);
    expect(menu.querySelector('#plyr-settings-1-speed')!.hasAttribute('hidden')).toBe(true);
  });

  it('opens on the main list after being closed on Speed, and drops a size left mid-animation', () => {
    const menu = closedOnQuality();
    menu.querySelector('#plyr-settings-1-home')!.setAttribute('hidden', '');
    menu.querySelector('#plyr-settings-1-speed')!.removeAttribute('hidden');
    menu.querySelector('[data-quality-menu]')!.setAttribute('hidden', '');
    prepareSettingsMenu(menu);
    expect(menu.querySelector('#plyr-settings-1-home')!.hasAttribute('hidden')).toBe(false);
    expect(menu.querySelector('#plyr-settings-1-speed')!.hasAttribute('hidden')).toBe(true);
    const frame = menu.firstElementChild as HTMLElement;
    expect(frame.style.width).toBe('');
    expect(frame.style.height).toBe('');
  });

  it('puts the menu on the glaze flyout, once', () => {
    const menu = closedOnQuality();
    prepareSettingsMenu(menu);
    prepareSettingsMenu(menu);
    expect(menu.className.split(' ').filter((c) => c === 'glass-flyout')).toHaveLength(1);
  });

  it('leaves a menu without Plyr panes alone apart from the glaze', () => {
    const menu = document.createElement('div');
    prepareSettingsMenu(menu);
    expect(menu.classList.contains('glass-flyout')).toBe(true);
  });
});
