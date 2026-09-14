// Platform capability map. For SHARED services/stores that both shells import.
// Tree-level desktop-vs-mobile selection happens once in main.tsx (lazy route
// split); components inside either tree should NOT consult this map, because
// they only ever run on their own platform.
import { IS_MOBILE } from './utils/platform';

export const features = Object.freeze({
  /** Popout windows, tray, window sizing/fullscreen orchestration. */
  multiWindow: !IS_MOBILE,
  /** convertFileSrc + download_and_cache_file local asset cache.
   *
   *  ON for mobile as of 2026-07-31. It was off with a note saying the Android
   *  asset protocol and cache dir were unwired; both turned out to be done
   *  already. `assetProtocol` is enabled in tauri.conf.json with `"scope":
   *  ["**"]`, and the cache dir resolves through `app_paths::mobile_base()`
   *  (the C1 app-data-dir fix), which is why `cache/universal/` has been
   *  writing on device all along. The flag was simply never revisited.
   *
   *  It mattered: with this off, every emote and badge image was re-fetched
   *  from the CDN on every render pass, forever, while the Cache settings panel
   *  claimed it was caching them.
   *
   *  If emotes ever render blank on Android, this is the first thing to flip
   *  back - consumers prefer `localUrl` over the CDN `url`, so a local URL that
   *  does not resolve fails to a broken image rather than falling back. */
  assetDiskCache: true,
  /** Per-row backdrop-filter atmosphere frost in chat (GPU-heavy on phones). */
  richAtmospheres: !IS_MOBILE,
  discordRpc: !IS_MOBILE,
  plugins: !IS_MOBILE,
  modTools: !IS_MOBILE,
});
