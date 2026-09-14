# Third-Party Notices

StreamNook itself is licensed under the terms in [LICENSE](LICENSE).

The components listed here are **not** covered by that license. Each is the
work of its own authors and remains under its own license. Those licenses are
permissive and are reproduced or linked below. Nothing in StreamNook's license
restricts your rights to these components.

## Vendored source

Source copied into this repository, rather than installed as a dependency.

| Component | Location | License |
| --- | --- | --- |
| shazamio-core (audio fingerprinting, by dotX12) | `src-tauri/src/services/song_id/` | MIT, see [`LICENSE-shazamio-core`](src-tauri/src/services/song_id/LICENSE-shazamio-core) |
| Fireworks simulator (by Caleb Miller) | `src/components/fireworks/fireworkSimulator.js` | MIT, see the file header |

## Bundled dependencies

Shipped inside StreamNook builds.

| Component | License |
| --- | --- |
| [Tauri](https://tauri.app/) | MIT or Apache-2.0 |
| [React](https://react.dev/) and React DOM | MIT |
| [Plyr](https://plyr.io/) | MIT |
| [HLS.js](https://github.com/video-dev/hls.js) | Apache-2.0 |
| [Framer Motion](https://www.framer.com/motion/) | MIT |
| [Zustand](https://github.com/pmndrs/zustand) | MIT |
| [Lucide](https://lucide.dev/) and [Phosphor](https://phosphoricons.com/) icons | ISC and MIT |
| [Fraunces](https://fonts.google.com/specimen/Fraunces) typeface | SIL Open Font License 1.1 |
| [supabase-js](https://github.com/supabase/supabase-js) | MIT |
| Rust crate dependencies | MIT, Apache-2.0, BSD, ISC, MPL-2.0, Zlib, Unicode-3.0 |
| npm runtime dependencies | MIT, Apache-2.0, ISC, BSD, MPL-2.0 |

For the complete, exact dependency lists see [`package.json`](package.json)
with [`package-lock.json`](package-lock.json), and
[`src-tauri/Cargo.toml`](src-tauri/Cargo.toml) with
[`src-tauri/Cargo.lock`](src-tauri/Cargo.lock).

## Build-time only

These are development dependencies. They are used to build StreamNook and are
**not** distributed in released binaries.

| Component | License |
| --- | --- |
| [sharp](https://sharp.pixelplumbing.com/) and libvips | Apache-2.0 and LGPL-3.0-or-later |
| [node-webpmux](https://github.com/ApeironTsuka/node-webpmux) | LGPL-3.0-or-later |

## Services and trademarks

StreamNook is an unofficial client. It is not affiliated with, endorsed by, or
sponsored by any of the services it connects to. Twitch, Kick, YouTube, 7TV,
BetterTTV, and FrankerFaceZ are trademarks of their respective owners, used
here only to identify those services. Use of StreamNook with any service
remains subject to that service's own terms.
