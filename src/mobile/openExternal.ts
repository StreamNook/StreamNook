// The implementation moved to src/utils/openExternal.ts (which carries the full
// explanation of why the shell plugin cannot open anything on Android).
//
// It moved because the SHARED chat components need it too: inline chat links
// (ChatMessage) and link-preview cards (LinkPreviewCard) were both still on
// `@tauri-apps/plugin-shell`, so every link tap on the phone was a dead tap.
//
// Nothing under src/components, src/utils or src/services may import from
// src/mobile - that would pull the mobile chunk into the desktop bundle - so the
// helper had to live in utils/. This file re-exports it rather than keeping a
// second copy, and the existing mobile callers (RewardsScreen, updateCheck) are
// unchanged.
export { openExternal } from '../utils/openExternal';
