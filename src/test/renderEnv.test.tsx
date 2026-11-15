// @vitest-environment happy-dom
// The environment every render test in this repo starts from. The tests here run the setup once on
// plain components before any real component leans on it: a render with its effect and click, the
// platform mock, and the exit of an AnimatePresence child.
import { act, useEffect, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { AnimatePresence, MotionGlobalConfig, motion } from 'framer-motion';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// happy-dom implements Element.animate, so an interrupted framer-motion enter or exit rejects with
// AbortError and vitest exits 1 with every test green; skipping animations avoids that.
MotionGlobalConfig.skipAnimations = true;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// The event module imports the core module itself, and a mock of core alone does not reach that
// import, so listen() would throw on transformCallback; both modules are mocked.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
// happy-dom's user agent follows the host, so the platform flags are explicit.
vi.mock('../utils/platform', () => ({
  IS_MOBILE: false,
  IS_ANDROID: false,
  IS_MAC: false,
  IS_LINUX: false,
  IS_WEBKITGTK: false,
  IS_MAIN_WINDOW: true,
  isMainWindow: () => true,
}));

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

let host: HTMLElement;
let root: Root;

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(listen).mockClear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

// Held in a constant: invokeParity.test.ts reads every quoted invoke name under src/, tests included.
const PROBE_COMMAND = 'probe';

function Probe() {
  const [text, setText] = useState('waiting');
  const [clicks, setClicks] = useState(0);
  useEffect(() => {
    let live = true;
    void invoke<string>(PROBE_COMMAND).then((answer) => {
      if (live) setText(answer);
    });
    void listen('probe-event', () => {});
    return () => {
      live = false;
    };
  }, []);
  return (
    <button data-testid="probe" onClick={() => setClicks((n) => n + 1)}>
      {`${text} ${clicks}`}
    </button>
  );
}

describe('render environment', () => {
  // Fails without the DOM environment, the act flag or either Tauri mock: the effect's invoke would
  // reach the real core module, and listen() would throw.
  it('renders, runs an effect through the mocked Tauri modules and answers a click', async () => {
    vi.mocked(invoke).mockResolvedValueOnce('ready');
    await act(async () => {
      root.render(<Probe />);
    });
    const button = host.querySelector('[data-testid="probe"]') as HTMLButtonElement;
    expect(button.textContent).toBe('ready 0');
    expect(vi.mocked(listen).mock.calls[0]?.[0]).toBe('probe-event');
    await act(async () => {
      button.click();
    });
    expect(button.textContent).toBe('ready 1');
  });

  // Fails when the platform module is not replaced by the explicit flags: the real module reads the
  // host's user agent, and before the link door lands it has no IS_MAIN_WINDOW at all.
  it('replaces the platform module with explicit flags', async () => {
    const platform: Record<string, unknown> = await import('../utils/platform');
    expect(platform.IS_MAIN_WINDOW).toBe(true);
    expect(platform.IS_MOBILE).toBe(false);
  });

  // The exit spike. Fails without MotionGlobalConfig.skipAnimations: the exit animation is then a
  // happy-dom Element.animate that is canceled with AbortError, so the run ends with an unhandled
  // rejection (and vitest exits 1) instead of finishing the exit.
  it('lets an AnimatePresence child leave once it is unmounted', async () => {
    const Panel = ({ show }: { show: boolean }) => (
      <AnimatePresence>
        {show && (
          <motion.div
            key="panel"
            data-testid="panel"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.18 }}
          />
        )}
      </AnimatePresence>
    );
    await act(async () => {
      root.render(<Panel show />);
    });
    expect(host.querySelector('[data-testid="panel"]')).not.toBeNull();
    await act(async () => {
      root.render(<Panel show={false} />);
    });
    expect(host.querySelector('[data-testid="panel"]')).toBeNull();
  });
});
