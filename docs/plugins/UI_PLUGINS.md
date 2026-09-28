# UI Plugins: in-app interface add-ons (runtime kind `ui`)

A second runtime kind alongside `process`. A UI plugin is a single JavaScript module the host loads into its own interface at runtime. It can contribute real interface: a title bar button, a floating panel, a column docked into an existing pane, command palette rows, bindable shortcuts, and its own popout OS windows. Installing one brings the whole feature; uninstalling removes every trace.

Both kinds share one manifest format, one marketplace, one signing chain, and one install and consent flow. They differ only in where the code runs:

| | `process` | `ui` |
|---|---|---|
| Runs as | A separate executable beside the app | A JavaScript module inside the app interface |
| Best at | Background behavior: long-running loops, own networking, work that must not depend on the interface being awake | Interface features: panels, buttons, windows, anything the user sees and touches |
| Can ship UI | Through a UI module (see Hybrid below) | Yes (contributes its own components) |
| Talks to the host via | JSON-RPC over stdio (PROTOCOL.md) | A JavaScript `api` object passed at load |
| Per-platform builds | One binary per OS | One artifact everywhere the app runs |

Choosing: if the feature is something the app *does in the background*, make it a `process` plugin. If it is something the user *sees and operates*, make it a `ui` plugin. If it is both (a background worker with its own native controls), make it a hybrid: a `process` plugin that also ships a UI module (see Hybrid plugins below).

## Manifest

Identical schema (MANIFEST.md), with the runtime block selecting the kind:

```toml
id          = "app.streamnook.lists"
name        = "Lists"
version     = "1.0.0"
author      = "StreamNook"
tier        = "A"
description = "User-curated reference lists: usernames, commands, titles."
host_min    = "7.0.0"

[runtime]
kind  = "ui"
entry = "dist/main.js"
```

For a pure `kind = "ui"` plugin:

- `entry` is the path of the bundled JavaScript module, relative to the plugin directory.
- `args` and `transport` are not used and should be omitted.
- `[capabilities]` lists (`events`, `host_methods`, `credentials`, `network`, `ui`) are wire-protocol concepts for `process` plugins and stay empty. The module contract below is the complete surface a UI plugin gets.
- `[contributes]` is likewise unused: UI plugin contributions are registered live through the `api` object, not declared in the manifest.

A hybrid plugin (a `process` sidecar that also ships a UI module) keeps `kind = "process"` and adds `ui_entry`; its `[capabilities]` and `[contributes]` blocks apply to the sidecar as normal. See Hybrid plugins below.

## Module contract

The entry file is a bundled ES module exporting:

```js
// Called once when the plugin loads in the main window. Register
// contributions here. May return a cleanup function.
export function activate(api) { /* ... */ }

// Optional. Called before the plugin is unloaded (disable, uninstall,
// update). The host also tears down every registration automatically,
// so this is for the plugin's own listeners and timers.
export function deactivate() { /* ... */ }

// Optional. Called in a popout OS window opened via api.windows.open.
// Returns the React component to render as that window's content
// (the host provides the window chrome, theme, and titlebar).
export function windowSurface(surfaceId, api) { return MyComponent; }
```

Loading: the host reads the bundle from the installed plugin directory and imports it when the plugin is enabled, at app start and on every enable. Disabling unloads it: `deactivate` runs, every registration is removed, and the module instance is discarded. Re-enabling evaluates a fresh instance.

Windows that host plugin contributions (the main window, the multi-chat popout) each load enabled UI plugins into their own context. `activate` runs per window; registrations only surface where a consumer for them exists.

## The `api` object

Everything a UI plugin can reach. Frozen, per plugin, passed to `activate` and `windowSurface`.

### `api.libs`

The host's own copies of shared libraries, so plugin bundles stay small and run on the same React tree:

- `api.libs.react` (also wired through the build shims, below)
- `api.libs.reactJsxRuntime`
- `api.libs.framerMotion`

Everything else a plugin needs (icons, state libraries) it bundles itself.

### `api.components`

Native components plugins should reuse instead of rebuilding:

- `api.components.Tooltip` (props: `content`, `side`, `delay`, `disabled`, `children`)
- Compact panel controls: `Toggle` (`enabled`, `onChange`, `ariaLabel`, `disabled`), `ChipList`, `FolderPicker`, `Slider`, `Select`, `ChannelList` (the channel search-picker), laid out with `SettingsSection`, `SettingsRow` and `SettingsBlock`.
- A full settings page in the Settings window's own look:
  - `SettingsPage` (`railKey`, `children`): scrolls on its own and shows the "On this page" rail on wide windows.
  - `PageSection` (`id`, `label`, `description`): one titled section; the rail lists every section by its label.
  - `PageRow` (`title`, `description`, `help`, `control`, `children`, `disabled`): the control sits on the right, and `children` go under the text.
  - `SegmentedSelect` (`value`, `options`, `onChange`).
  - `PillList` (`items`, `numbered`, `placeholder`, `onChange`): a short list of names edited in place.
  - `InlineSlider` (`value`, `min`, `max`, `step`, `label`, `format`, `disabled`, `onChange`).
  - `SubControls` wrapping `SubControl` (`title`, `control`, `disabled`): the settings under a choice such as Custom.

Build a page's markup from these rather than your own classes: the host stylesheet only contains the classes the host itself uses, so a plugin's own Tailwind classes may not exist in it.

### `api.ui`

- `api.ui.registerTitleBarButton({ id, tooltip, Icon, onClick, useIsActive? })`
  Adds a button to the title bar's action cluster, rendered in native style. `Icon` is a component receiving `{ size }`. `useIsActive` is an optional React hook returning whether the button shows the accent tint (read it from the plugin's own state).
- `api.ui.registerOverlay({ id, Component })`
  Mounts `Component` at the app root, above the main layout. The component owns its visibility (render `null` when closed), positioning, and animation. This is how a plugin ships a floating panel.
  Each button is its own component, so `useIsActive` and `useIsVisible` are real React hooks and may read any store.
- `api.ui.registerSlot(slotId, { id, label, Icon, Component })`
  Fills a named slot a host feature exposes. The owning feature decides how slot contributions render. Slot ids are namespaced and host-defined, like hook ids in HOOKS.md.

A title-bar button takes an optional `useIsVisible` hook alongside `useIsActive`. Return `false` to hide the button. This is how a plugin lets the *user* decide whether its button rides in the title bar: keep the choice in one of the plugin's own settings and read it from `useIsVisible`, so toggling the setting adds or removes the button live. The Lists plugin does exactly this (a "Title bar button" switch in its settings panel; the panel author picks the icon, the user picks whether it shows).

### `api.commands`

- `api.commands.registerKeybinding({ id, label, description?, category?, defaultBindings, keywords?, run })`
  Adds a bindable command: it dispatches globally, appears in the Keybindings settings, and user rebinds persist under its `id`. A plugin replacing a former core feature may reuse the historical command id so existing user rebinds keep working.
- `api.commands.registerPaletteItems(provider)`
  `provider` is called on each palette open and returns rows: `{ id, section?, title, subtitle?, keywords?, icon?, run }`.

### `api.settings`

- `api.settings.registerPanel(Component)`
  Registers the plugin's own settings component. It renders on the plugin's card in the plugins page, under a gear that appears while the plugin is enabled. This is the in-process equivalent of a `process` plugin's host-rendered panel, except the plugin ships the actual UI: build it from `api.components` and your own controls so it reads as part of the app. Persist values yourself (under a `streamnook.<feature>.` localStorage key) and read them reactively in the contributions that depend on them (a title-bar button's `useIsVisible`, an overlay's contents, and so on), so a settings change takes effect live.

### `api.windows`

- `api.windows.open({ surface, title, width?, height?, minWidth?, minHeight? })`
  Opens (or focuses) a popout OS window for this plugin. The host renders the standard frameless titlebar (icon, title, keep-on-top pin, minimize, close), applies the user's theme, and mounts the component returned by the module's `windowSurface(surface, api)`. One window per `(plugin, surface)`; reopening focuses it.

### `api.events`

- `api.events.emit(name, payload)` and `api.events.listen(name, handler) -> Promise<unlisten>`
  App-wide events that cross window boundaries, for state sync between the main window and popouts. Names should be prefixed with the plugin's id unless keeping a historical name for data continuity. Listeners are cleaned up automatically at unload.

### `api.chat`

- `api.chat.useHasTarget()` React hook: true when a chat compose box exists in this window (a stream is being watched or the multi-stream grid is up).
- `api.chat.insertText(text) -> boolean` inserts into the chat compose box at the caret; false when none is mounted.

### `api.storage`

UI plugins read and write `localStorage` directly (it is shared across the app's windows, which is what makes popout sync work). Keys must be prefixed: `streamnook.<feature>.` for a feature the plugin owns. There is no separate storage API.

### `api.log`

`api.log.debug / warn / error`, prefixed with the plugin id in the app log.

## Slot catalog

Slots host features currently expose to UI plugins:

- `modlogs.dock`: a column inside the Moderator Logs pane, side by side with the log columns. The pane shows one toggle button per contribution (using its `Icon` and `label`) and persists each toggle at `streamnook.modlogs.dock.<id>`.

## Building a UI plugin

Bundle to a single ES module with the shared libraries aliased to the host's copies. With esbuild:

```js
import { build } from 'esbuild';
build({
  entryPoints: ['src/index.tsx'],
  bundle: true,
  format: 'esm',
  outfile: 'dist/main.js',
  jsx: 'automatic',
  alias: {
    'react': './shims/react.cjs',
    'react/jsx-runtime': './shims/jsx-runtime.cjs',
    'framer-motion': './shims/framer-motion.cjs',
  },
});
```

Each shim re-exports the host copy, e.g. `shims/react.cjs`:

```js
module.exports = globalThis.__STREAMNOOK_HOST_LIBS__.react;
```

Sharing React this way is required, not an optimization: contributed components render inside the host's React tree, and hooks only work when both sides run the same React instance. Everything else (icons, a state library) is bundled into the plugin normally.

## Hybrid plugins (process + ui)

A `process` plugin can also ship a UI module by adding `ui_entry` to its `[runtime]` block:

```toml
[runtime]
kind      = "process"
entry     = "worker.exe"      # the sidecar
ui_entry  = "dist/main.js"    # the in-app UI module
transport = "stdio"
```

When the plugin is enabled, the host runs both: it spawns the sidecar and the frontend loads the UI module, exactly as for a standalone ui plugin. When it is disabled, the sidecar shuts down and the UI unloads. One plugin, one install, one toggle.

The two halves are separate programs, so they talk through the existing hook system (HOOKS.md), not directly. The UI module invokes the sidecar's actions and reads its status with the same calls the host's own UI uses:

- `invoke('plugins_invoke_action', { action, args })` to drive the sidecar.
- `invoke('plugins_provides', { feature })` to check the sidecar is backing a feature.
- `listen('plugin://status', ...)` to receive the sidecar's status pushes (filter by the event's `plugin_id`).

This is how a background worker contributes native controls: the sidecar does the work and owns the data; the UI module contributes the buttons, the status display, and the settings panel, and feels like part of the app while the plugin is on and disappears when it is off. The UI module shares the host's React and uses the whole `api` above (title-bar buttons, overlays, slots, palette rows, keybindings, windows, and a settings panel) on top of the action and status calls.

## Trust model

A UI plugin runs inside the app's interface with the same reach as the app's own interface code. That is the same trust bar as a `process` plugin, which runs native code on the machine. What protects users is unchanged across both kinds: curation into a signed index, artifact signatures and hashes verified before anything runs, the install consent dialog, and one-click disable and uninstall.

## Versioning

The module contract and `api` surface above are version 1 of the UI runtime. Additive changes (new `api` members, new slots, new optional fields) do not bump it; removing or changing existing members does. Plugins should feature-detect optional members (`if (api.ui.registerSlot)`).
