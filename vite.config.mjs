import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { visualizer } from 'rollup-plugin-visualizer';

// Paths (substring match on the module path) the React Compiler compiles.
// Kept deliberately explicit: the chat row and list, the settings panels,
// and the shared switch. See the compiler entry in plugins below.
const REACT_COMPILER_SOURCES = [
  '/src/components/ChatMessage.tsx',
  '/src/components/ChatMessageList.tsx',
  '/src/components/PlayerStatsOverlay.tsx',
  '/src/components/chat/',
  '/src/components/multichat/',
  '/src/components/settings/',
  '/src/components/ui/',
];
// Inside the list but not yet clean under eslint-plugin-react-hooks 7.1
// (optimistic grants in effects, a DOM lookup after mount). Kept out until
// those are reworked; eslint.config.js mirrors this boundary.
const REACT_COMPILER_EXCLUDES = [
  '/src/components/settings/ProfileSettings.tsx',
  '/src/components/settings/ProfileOverview.tsx',
  '/src/components/settings/PluginsSettings.tsx',
  // The incremental multi-channel merge now lives in hooks/useBlendedChatSource
  // (shared with the main chat panel), which is outside the compiled paths above.
  // This exclusion still has to stay: the pane keeps its own refs mutated during
  // render (messagesRef, the pause anchors) — a deliberate cache the compiler's
  // rules forbid, and moving those into the store is design work, not mechanics.
  '/src/components/multichat/BlendedChatPane.tsx',
];

// A physical Android device cannot reach the host's localhost, so `tauri android
// dev` rewrites devUrl to the interface IP it detects and exports that IP as
// TAURI_DEV_HOST for this config to bind to. Without honouring it, Tauri waits
// forever on "Waiting for your frontend dev server to start".
// Unset on desktop `tauri dev`, where `host: false` keeps the current
// localhost-only binding, so desktop behaviour is unchanged.
const devHost = process.env.TAURI_DEV_HOST;

// Android runs on its own port so it can never collide with a desktop `tauri dev`
// (or a leftover preview server) already holding 1420. src-tauri/tauri.android.conf.json
// sets SN_DEV_PORT=1430 and points devUrl at it; desktop keeps the default.
// HMR gets port+1, which must differ per platform for the same reason.
const devPort = Number(process.env.SN_DEV_PORT ?? 1420);

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [
    react({
      babel: {
        plugins: [
          // React Compiler, scoped to the rollout allowlist below. Inside it the
          // compiler infers components and hooks and memoizes what it can prove;
          // anything it cannot (dynamic import(), try/finally, a disabled React
          // lint rule) is skipped, never a build error. Files outside the list
          // are untouched. Widen the list as files clear
          // eslint-plugin-react-hooks 7.1.1, which enforces the same rules.
          ['babel-plugin-react-compiler', {
            compilationMode: 'infer',
            panicThreshold: 'none',
            sources: (filename) => {
              const f = filename.replace(/\\/g, '/');
              return REACT_COMPILER_SOURCES.some((p) => f.includes(p)) && !REACT_COMPILER_EXCLUDES.some((p) => f.includes(p));
            },
          }],
        ],
      },
    }),
    // Bundle breakdown on demand: ANALYZE=1 npm run build writes stats.html.
    ...(process.env.ANALYZE ? [visualizer({ filename: 'stats.html', gzipSize: true })] : []),
  ],
  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  clearScreen: false,
  server: {
    host: devHost || false,
    port: devPort,
    strictPort: true,
    // HMR needs its own explicit host when serving a device over the network;
    // the default derives the websocket URL from `location`, which is the phone.
    hmr: devHost ? { protocol: 'ws', host: devHost, port: devPort + 1 } : undefined,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    // The only runtime is Tauri's bundled WebView2, so target its engine
    // instead of a generic browser matrix.
    target: 'chrome110',
    chunkSizeWarningLimit: 900,
    rollupOptions: {
      output: {
        manualChunks: {
          // React 19 moved the DOM renderer out of react-dom's main entry into
          // react-dom/client; without listing the subpath the 180 KB renderer
          // lands in the entry chunk and vendor-core shrinks to a 4 KB shim.
          'vendor-core': ['react', 'react-dom', 'react-dom/client', 'zustand'],
          'vendor-hls': ['hls.js', 'plyr'],
          'vendor-motion': ['framer-motion'],
          'vendor-tauri': [
            '@tauri-apps/api',
            '@tauri-apps/plugin-shell',
            '@tauri-apps/plugin-dialog',
          ],
        }
      }
    }
  }
})
