// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MotionGlobalConfig } from 'framer-motion';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import type { TwitchCategory } from '../types';

MotionGlobalConfig.skipAnimations = true;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => null),
  convertFileSrc: (path: string) => path,
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
vi.mock('../utils/platform', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  IS_MOBILE: false,
  IS_LINUX: false,
  IS_MAIN_WINDOW: true,
  isMainWindow: () => true,
}));

import NavFlipper from './NavFlipper';
import { useAppStore } from '../stores/AppStore';

const initialState = useAppStore.getState();

let root: Root | null = null;
let host: HTMLElement;

const forwardLabelFor = async (exited: TwitchCategory | null) => {
  // Home is up on the browse tab, and the category the person stepped back out of waits to be re-entered.
  useAppStore.setState({
    isHomeActive: true,
    streamUrl: null,
    homeActiveTab: 'browse',
    homeSelectedCategory: null,
    homeProviderCategory: null,
    homeLastExitedCategory: exited,
  } as never);
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => {
    root!.render(<NavFlipper />);
  });
  const buttons = Array.from(host.querySelectorAll('button'));
  return buttons[1]?.getAttribute('aria-label');
};

beforeEach(() => {
  useAppStore.setState(initialState, true);
});

afterEach(async () => {
  if (root) {
    await act(async () => {
      root!.unmount();
    });
    root = null;
  }
  host?.remove();
});

describe('the forward step back into a category', () => {
  it('says "Back to category" for a copy that has no name yet', async () => {
    // Without the fallback this reads "Back to " and nothing after it.
    expect(await forwardLabelFor({ id: '509658', name: '', box_art_url: '' })).toBe('Back to category');
  });

  it('says the name once the copy has one', async () => {
    expect(await forwardLabelFor({ id: '509658', name: 'Rust', box_art_url: '' })).toBe('Back to Rust');
  });
});
