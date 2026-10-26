// The watch layer.
//
// Portrait full: 16:9 player band + the desktop-identity chat header (blurred
// overlay with stream info, pinned strip, HypeTrainBanner) + chat, with the
// poll and prediction cards mounted over the chat like desktop.
// Landscape: immersive full-bleed player (system bars hidden) + chat toggle.
//
// SHRINKING IS ONE CONCEPT WITH TWO BACKINGS. The viewer sees one thing, a
// small floating player, reached by dragging the band down, by the minimize
// control, or by pressing back:
//   - inside the app it is the mini box, so the tab shell stays live behind it
//     and browsing continues with the stream playing (tap to expand, X to
//     close);
//   - leave the app and the OS takes over with system PiP, which floats over
//     every app but necessarily takes StreamNook's own UI off screen, since a
//     single activity IS the PiP window.
// Nothing in the UI calls system PiP directly. It is what leaving the app does.
import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import {
  AnimatePresence,
  animate,
  motion,
  useMotionValue,
  useReducedMotionConfig,
  useTransform,
} from 'framer-motion';
import { SpeakerHigh, SpeakerSlash, X } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { useRoomPins } from '../chat/useRoomPins';
import { useChatTabsStore } from '../chat/chatTabsStore';
import { useWindowShape } from '../ui/useWindowShape';
import { usePageState } from '../ui/usePageState';
import {
  readWatchLayout,
  readWatchSplit,
  writeWatchLayout,
  writeWatchSplit,
  type WatchLayout,
} from '../watch/watchLayout';
import { SplitHandle } from '../watch/SplitHandle';
import { MobilePlayer } from '../player/MobilePlayer';
import { CastCard, CastEndedCard, TvCard } from '../cast/CastCard';
import { castOwnsStream, isCastable, useCastState } from '../cast/castState';
import { useTvScreenState } from '../cast/tvScreenState';
import { MobileOfflineChin } from '../player/MobileOfflineRoom';
import { toggleLiveMute, useLiveVideoMuted, useLiveVideoPaused } from '../player/liveVideo';
import { miniLayerTransform } from '../watch/miniLayer';
import { keepsAudioAfterPip } from '../player/lockScreenAudio';
import { MobileChatPane } from '../chat/MobileChatPane';
import {
  CHAT_TAB_STRIP_H,
  useActiveChatChannelId,
  useChatTabsVisible,
  useViewingStreamChat,
} from '../chat/ChatTabStrip';
import { useMentionsStore } from '../mentions/mentionInbox';
import { MentionContextCard } from '../mentions/MentionContextCard';
import { DropProgressBar } from '../watch/DropProgressBar';
import { PinnedBanner } from '../watch/PinnedBanner';
import { usePinsOpen } from '../watch/pinsOpen';
import HypeTrainBanner from '../../components/HypeTrainBanner';
import PollOverlay from '../../components/PollOverlay';
import PredictionOverlay from '../../components/PredictionOverlay';
import { ChatOverlayStack } from '../../components/chat/ChatOverlayStack';
import { useChatTopStack } from '../../components/chat/chatTopStack';
import {
  consumePipClosed,
  getMediaVolume,
  getScreenBrightness,
  isInPip,
  setImmersive,
  holdScreenOn,
  setMediaVolume,
  setPipEligible,
  setPipSourceRect,
  setPlayerOrientation,
  setPreferredRefreshRate,
  setScreenBrightness,
} from '../nativeBridge';
import { DialIndicator, type DialAxis, type DialReading } from '../watch/DialIndicator';
import { dialCanStart } from '../watch/dialZone';
import { ChatOverlayChrome } from '../watch/ChatOverlayChrome';
import type { FullscreenChatSettings } from '../../types';
import { hapticStep } from '../ui/haptics';
import { readInsets, type ResolvedInsets } from '../nativeInsets';
import { swallowTrailingClick } from '../ui/trailingClick';
import { isTwitchStream } from '../../utils/streamProvider';

const MINI_W = 200;
const MINI_H = Math.round((MINI_W * 9) / 16);
const EDGE_GAP = 12;
// The mini box's side gutter: the pages' own 16 px, so its edge lines up with
// the cards under it. Used by the clamp, the resting spot and the snap alike.
const MINI_GUTTER = 16;
// Clearance so the resting mini player never sits under the floating pill bar
// (56 px, icons only, riding 14 px above the gesture strip).
const BOTTOM_GAP = 104;

// Where the mini player is ALLOWED to be.
//
// This exists because of a bug with a very specific shape: minimize to the mini
// player, then swipe up to leave the app, and the box flew to the top of the
// screen, clipped off, and was never reachable again - and because the WebView
// is never torn down (unconsumed back is moveTaskToBack, the home swipe
// auto-enters PiP), it stayed stranded across what looks to the viewer like an
// app restart.
//
// Three defects fed it:
//   1. The release snap clamped y from ONE SIDE only (a bare Math.min), so any
//      short viewport drove y negative. The drag path was two-sided; the release
//      path was not.
//   2. The bounds were hardcoded 12 / 104 and inset-blind, so at the top the
//      56x56 close-X quadrant sat under the status bar and could not be tapped.
//   3. Nothing re-clamped the stored position when the viewport CHANGED, and a
//      gesture-nav home swipe auto-enters PiP, which resizes the activity to
//      roughly 213x120 CSS px MID-GESTURE. The snap then ran against that
//      viewport: y = min(pos.y, 120 - 113 - 104) = -97.
//
// So there is now ONE clamp, every writer goes through it, and it takes insets.
function clampMini(
  x: number,
  y: number,
  vw: number,
  vh: number,
  ins: ResolvedInsets,
  floor: number = BOTTOM_GAP,
): { x: number; y: number } {
  // Clear of the BACK strips at the sides, and of the status bar plus the home /
  // quick-switch strips top and bottom. An app cannot opt out of the home
  // gesture at all, so simply not overlapping it is the only thing that works.
  const left = MINI_GUTTER + Math.max(ins.left, ins.gestureLeft);
  const right = MINI_GUTTER + Math.max(ins.right, ins.gestureRight);
  const top = EDGE_GAP + Math.max(ins.top, ins.gestureTop);
  const bottom = Math.max(floor, ins.bottom + ins.gestureBottom + EDGE_GAP);

  // Math.max applied LAST, so that on a viewport too small to satisfy both
  // bounds the box lands at the top-left edge rather than off-screen. Getting
  // this order wrong is exactly how defect 1 above shipped.
  return {
    x: Math.max(left, Math.min(vw - MINI_W - right, x)),
    y: Math.max(top, Math.min(vh - MINI_H - bottom, y)),
  };
}

const TAP_SLOP_PX = 8;
// Travel before the shrink is fully previewed. A fraction of the screen, not a
// fixed pixel count, so the gesture feels the same on a compact phone and an
// unfolded Fold.
const shrinkTravel = (h: number) => Math.max(120, Math.min(280, h * 0.28));
const COMMIT_FRACTION = 0.45;
// A downward flick commits regardless of distance. CSS pixels per millisecond.
const FLING_VY = 0.6;
const SPRING = { type: 'spring', stiffness: 420, damping: 36, mass: 0.7 } as const;
// The phone mini player's move (scale model): Xtra's 250 ms, eased at both ends.
const TWEEN = { duration: 0.25, ease: 'easeInOut' } as const;
// Backstop for the layer coming to rest. The spring lands in about 450 ms; an
// update with nothing to animate never reports completion, so this clears the
// growing state and the chat hold after it instead.
const SETTLE_FALLBACK_MS = 900;

const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

export const WatchScreen: React.FC = () => {
  const isLoading = useAppStore((s) => s.isLoading);
  const streamUrl = useAppStore((s) => s.streamUrl);
  const currentStream = useAppStore((s) => s.currentStream);
  const restartStream = useAppStore((s) => s.restartStream);
  const currentHypeTrain = useAppStore((s) => s.currentHypeTrain);
  const exitStream = useAppStore((s) => s.exitStream);
  const playerMode = useMobileNavStore((s) => s.playerMode);
  const setPlayerMode = useMobileNavStore((s) => s.setPlayerMode);
  const shape = useWindowShape();
  const [landscapeChat, setLandscapeChat] = useState(false);
  // A large screen shows chat by default (beside or under the video), so its
  // toggle runs the other way: it HIDES chat, and the video takes the screen.
  const [largeChatHidden, setLargeChatHidden] = useState(false);
  const [watchLayout, setWatchLayout] = useState<WatchLayout>(readWatchLayout);
  const [chatSplit, setChatSplit] = useState<number | null>(readWatchSplit);
  const [pip, setPip] = useState(false);
  // A pin opens or folds per channel: each channel starts as Chat > Pinned
  // Messages > Collapsed says, and a tap holds until the chat switches.
  const pins = usePinsOpen();
  // The poll, prediction and drop cards: one stack in the chat column's flow,
  // laid out by Rust against the column's height. Opened together on a short
  // column (a foldable's side-docked chat) they used to leave no room to read
  // or type, and the drop card painted over the poll's fold control.
  const [chatColumnEl, setChatColumnEl] = useState<HTMLDivElement | null>(null);
  const topStack = useChatTopStack(chatColumnEl);
  const chatTabsVisible = useChatTabsVisible();
  // The hype train and pinned message float over chat just under the stack:
  // where the stack ends inside the chat column (its offset parent), or null
  // while it is empty. Re-read when the tab strip above it comes or goes,
  // which moves it without resizing it.
  const [topStackEl, setTopStackEl] = useState<HTMLDivElement | null>(null);
  const [topStackBottom, setTopStackBottom] = useState<number | null>(null);
  useEffect(() => {
    if (!topStackEl) return;
    const read = () =>
      setTopStackBottom(topStackEl.offsetHeight > 0 ? topStackEl.offsetTop + topStackEl.offsetHeight : null);
    read();
    const ro = new ResizeObserver(read);
    ro.observe(topStackEl);
    return () => ro.disconnect();
  }, [topStackEl, chatTabsVisible]);
  const chatTabsOpen = useChatTabsStore((s) => s.tabs.length > 0);
  const viewingStreamChat = useViewingStreamChat();
  // Hype train, drops, polls and predictions are Twitch contracts keyed by a
  // numeric user_id; a Kick id would name an unrelated Twitch channel.
  const streamScoped = viewingStreamChat && isTwitchStream(currentStream);
  const chatChannelId = useActiveChatChannelId();
  // An opened mention older than its chat's history, pinned over that chat.
  const mentionCard = useMentionsStore((s) => s.contextCard);
  const activeChatRoom = useChatTabsStore((s) => s.activeChannel);
  const showMentionCard = !!mentionCard && mentionCard.room === activeChatRoom;
  // The viewer picked the audio-only rendition: there is no picture to float.
  const audioOnly = useAppStore((s) => s.activeQuality === 'audio_only');
  // The landscape chat overlay shares its knobs with the desktop overlay:
  // opacity, width and side are one preference however big the screen is.
  const fsChat = useAppStore((s) => s.settings.fullscreen_chat);
  // The poll and prediction cards follow the same settings as desktop chat:
  // each can be turned off, and the user picks which one sits on top (read by
  // the stack's layout).
  const showPolls = useAppStore((s) => s.settings.show_polls) !== false;
  const showPredictions = useAppStore((s) => s.settings.show_predictions) !== false;
  // The dial being turned right now (brightness or volume), for the readout.
  const [dial, setDial] = useState<DialReading | null>(null);
  const dialHide = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Whether this session ever overrode the window brightness, so the reset
  // when playback ends is only sent if there is something to reset.
  const brightnessTouched = useRef(false);
  const [viewport, setViewport] = useState(() => ({
    w: window.innerWidth,
    h: window.innerHeight,
  }));
  // Through the clamp even on first mount, so the default resting spot respects
  // the gesture strips on a device whose insets differ from the hardcoded gaps.
  const [miniPos, setMiniPos] = useState(() =>
    clampMini(
      window.innerWidth - MINI_W - MINI_GUTTER,
      window.innerHeight - MINI_H - BOTTOM_GAP,
      window.innerWidth,
      window.innerHeight,
      readInsets(),
    ),
  );
  // How far through the shrink preview the finger currently is: 0 is full, 1 is
  // the mini box. This is what makes a HALF drag a real, reversible state
  // instead of the no-op it used to be.
  const [shrink, setShrink] = useState(0);
  const [dragging, setDragging] = useState(false);
  const suppressClick = useRef(false);
  const bandRef = useRef<HTMLDivElement | null>(null);
  // One gesture record for both drags. `kind` is fixed at pointerdown, and
  // `progress` rides along so pointerup reads it without a stale closure.
  const gesture = useRef<{
    id: number;
    /** shrink: portrait drag-down to the mini box; move: dragging the mini
     *  box; dial: a vertical swipe over landscape video turning brightness
     *  (left half) or volume (right half). */
    kind: 'shrink' | 'move' | 'dial';
    x: number;
    y: number;
    px: number;
    py: number;
    lastY: number;
    lastT: number;
    vy: number;
    progress: number;
    moved: boolean;
    /** Absent when the press began in an edge strip (dialZone): that
     *  gesture is a tap or nothing, never a dial. */
    dial?: {
      axis: DialAxis;
      /** The level when the finger landed; the drag is relative to it. */
      start: number;
      /** Pixels of travel for the full 0..1 swing. */
      travel: number;
      /** Last tenth crossed, for the haptic tick. */
      lastStep: number;
      /** Last hundredth pushed to the bridge, so a still finger costs nothing. */
      lastPushed: number;
    };
  } | null>(null);

  // One visibility flag for the whole layer (computed here because the mode
  // derivation below needs it; consumed by the AnimatePresence at the bottom).
  const show = !!streamUrl || isLoading;

  // A FRESH open always enters full, from the very first painted frame. The
  // mode outlives a close (a stream closed from its mini box leaves 'mini' in
  // the nav store), and the "newly started stream opens full" sync below is an
  // effect that waits for the chat channel to resolve — so without this the
  // layer MOUNTED as the mini box, showed the loading logo in it, then
  // animated up to full: tapping a stream card is not a request for a mini
  // box. Render-time state adjustment (the drill-in screens' pattern), and
  // `forceFull` holds until the nav store has actually caught up, so no
  // committed frame ever derives mini from the stale mode.
  const [prevShow, setPrevShow] = useState(false);
  const [forceFull, setForceFull] = useState(false);
  // The value this pass settles on, read below in the SAME pass. A render that
  // sets state runs again before it commits, but other adjustments made from
  // the old value are kept, so `mini` must never read the stale one.
  let forceFullNow = forceFull;
  if (show && !prevShow) {
    setPrevShow(true);
    forceFullNow = true;
  } else if (!show && prevShow) {
    setPrevShow(false);
    forceFullNow = false;
  } else if (forceFull && playerMode === 'full') {
    forceFullNow = false;
  }
  if (forceFullNow !== forceFull) setForceFull(forceFullNow);
  useEffect(() => {
    if (forceFull) setPlayerMode('full');
  }, [forceFull, setPlayerMode]);

  const mini = !forceFullNow && playerMode === 'mini';
  const watching = !!streamUrl && streamUrl !== 'offline';

  // The mini box growing back into the full player, by any route (a tap on the
  // box, a mention, a new stream): from the commit where a box that was ON
  // SCREEN stops being mini until the layer's spring lands. Never a fresh open,
  // which shows no box, and never a close. While it is set a shield takes every
  // press (bottom of the layer), so nothing in the moving layer is pressed; where
  // the layer still animates its rect (tablets, landscape) chat also stays
  // hidden until it lands, so the spring never lays chat out.
  const miniOnScreen = show && mini;
  const [prevMiniOnScreen, setPrevMiniOnScreen] = useState(false);
  const [landing, setLanding] = useState(false);
  if (miniOnScreen !== prevMiniOnScreen) {
    setPrevMiniOnScreen(miniOnScreen);
    setLanding(!miniOnScreen && show);
  }
  // Chat's full-size box, held from the first frame of a shrink drag until the
  // layer is at rest again, so the shrinking layer crops chat instead of
  // re-wrapping every row at a new width each frame.
  const [chatFreeze, setChatFreeze] = useState<{ w: number; h: number } | null>(null);
  // One commit with no spring, after the phone turns between the two geometry
  // models (see scaleModel); set where the model is derived, cleared here.
  const [modelJump, setModelJump] = useState(false);
  const settle = useCallback(() => {
    setLanding(false);
    setChatFreeze(null);
    setModelJump(false);
  }, []);
  useEffect(() => {
    if (dragging || (!landing && !chatFreeze)) return;
    const t = setTimeout(settle, SETTLE_FALLBACK_MS);
    return () => clearTimeout(t);
  }, [dragging, landing, chatFreeze, settle]);

  // A Cast device playing this stream takes the player's place; the phone is
  // the remote and chat carries on. Rust decides everything (cast_session.rs);
  // this only picks which card to draw.
  const cast = useCastState();
  const castOwns = castOwnsStream(cast, currentStream);
  // When a session ends with this stream on the TV, the stream the phone holds
  // is either a cast placeholder or a relay Rust stopped. Until the viewer
  // picks something else the band offers "Watch on this phone" rather than
  // starting the phone's player by itself (the session may have ended in a
  // pocket). Keyed on the session ending, not on castOwns falling: opening a
  // stream the TV cannot take (YouTube) also drops castOwns, and that one must
  // simply play here.
  const castConnected = cast.phase === 'casting';
  const [castEndedUrl, setCastEndedUrl] = useState<string | null>(null);
  const [prevCastConnected, setPrevCastConnected] = useState(castConnected);
  if (castConnected !== prevCastConnected) {
    setPrevCastConnected(castConnected);
    setCastEndedUrl(!castConnected && isCastable(currentStream) ? streamUrl : null);
  }
  const castEnded =
    !castOwns && !!streamUrl && (streamUrl === castEndedUrl || streamUrl.startsWith('cast://'));
  // Playing on THIS screen: what keeps it awake, at 60 Hz, and PiP-eligible.
  // TV screen mode: while the phone mirrors to a TV, Rust plays the stream
  // full-frame there (TvScreen.kt) and the band shows the remote card.
  const tv = useTvScreenState();
  const tvOwns = watching && tv.active && !castOwns;
  const playingHere = watching && !castOwns && !castEnded && !tvOwns;
  const channelId = currentStream?.user_id;

  // Side by side is decided by AVAILABLE WIDTH, not by orientation. An unfolded
  // Z Fold is about 840x757 — an aspect ratio near 1.1 — so asking "portrait or
  // landscape" there gives an answer that means nothing, and the old check handed
  // a big tablet-shaped screen the phone's stacked layout.
  // A vertical hinge says WHERE to split if we split left/right. It does not say
  // that we should. The crease on an unfolded Fold sits at the middle, so
  // honouring it gives the player ~420dp of width: a 236dp-tall video stranded
  // in a 757dp-tall column, which reads as a small picture on a mostly black
  // pane. Stacking that same screen gets a 472dp video with chat still 285dp
  // tall. So choose the arrangement that gives the VIDEO the most height while
  // leaving chat something usable, instead of assuming a big screen wants
  // columns.
  const MIN_CHAT_W = 300;
  const MIN_CHAT_H = 240;
  const sideVideoH = (shape.splitX * 9) / 16;
  const stackVideoH = (shape.w * 9) / 16;
  const sideFits = shape.w - shape.splitX >= MIN_CHAT_W;
  const stackFits = shape.h - stackVideoH >= MIN_CHAT_H;
  // ...but only as a DEFAULT. Both arrangements are legitimate on a near-square
  // screen: columns trade picture size for full-height chat, stacked trades
  // pillarbox bars for a big picture. That is the viewer's call, so `auto` is
  // just the starting point and an explicit choice always wins.
  const autoColumns = sideFits && (!stackFits || sideVideoH >= stackVideoH);
  // Offer the switch only where both actually work; below that there is nothing
  // to choose between.
  const canChooseLayout = shape.largeScreen && sideFits && stackFits;
  // Only a genuinely large screen re-decides (smallest side, see useWindowShape:
  // an ordinary phone is 900dp wide on its side and is still a phone). A phone
  // in landscape is after immersive video, so chat stays opt-in behind the
  // fullscreen toggle there. Deciding this by WIDTH is what put chat on the
  // right of every landscape phone with no way to dismiss it.
  //
  // On a phone the landscape chat toggle takes a column beside the picture by
  // default: the video gets the rest of the screen to itself, edge to edge.
  // "Over the video" floats the column over the picture instead, translucent,
  // at the width and opacity the overlay settings say.
  const phoneOverlayChat = !shape.largeScreen && (fsChat?.phone_layout ?? 'beside') === 'overlay';
  const twoColumns =
    shape.largeScreen
      ? largeChatHidden
        ? false
        : watchLayout === 'auto'
        ? autoColumns
        : watchLayout === 'columns' && sideFits
      : landscapeChat && !phoneOverlayChat;
  // Full-bleed video on its side: a phone always, a tablet once chat is hidden.
  // Leaving the tablet out entirely is what made its chat button do nothing.
  const immersiveLandscape = shape.twoPane && (!shape.largeScreen || largeChatHidden);
  // Where the chat button appears and what it flips. A large screen gets it
  // whenever it is on its side, in either layout, so stacked can go full too.
  const largeToggle = shape.largeScreen && shape.twoPane && shape.splitY == null;
  // Tabletop: the phone is bent across a horizontal hinge. The bend is the
  // natural boundary between picture and chat, so the band ends on it and
  // chat takes the lower half. Posture beats the remembered split and beats
  // the columns arrangement; a picture across a bend is the one thing this
  // posture cannot show.
  const tabletop = shape.splitY != null && !mini && !pip;
  const sideBySide = !tabletop && shape.twoPane && !mini && !pip && (twoColumns || immersiveLandscape);
  const chatBeside = sideBySide && twoColumns;
  const chatOverlay = sideBySide && immersiveLandscape && landscapeChat && phoneOverlayChat;
  // Only the stacked portrait band has room below it for the offline card;
  // everywhere else the card stays over the art.
  const offlineChin = !sideBySide && !tabletop && !mini && !pip;
  // A phone's "Beside the video" is the overlay column taken out of the
  // picture: same edge, same width and the same drag handle, still immersive,
  // but the video gets the rest of the screen to itself instead of sitting
  // centred under an opaque column with bars at its far edge.
  const phoneBeside = chatBeside && !shape.largeScreen;
  const overlaySide: 'left' | 'right' = fsChat?.side === 'left' ? 'left' : 'right';
  // The overlay's own controls change width and see-through IN PLACE (a drag on
  // its edge, a slider on its strip). The live value leads while the finger is
  // down and until the saved setting catches up; the setting is what survives.
  const [overlayWidthLive, setOverlayWidthLive] = useState<number | null>(null);
  const [overlayOpacityLive, setOverlayOpacityLive] = useState<number | null>(null);
  // updateSettings applies the new value to the store only after the save
  // resolves, so the live value is released THEN: never a frame on the old
  // saved value between the finger lifting and the write landing.
  const writeOverlay = useCallback(async (patch: Partial<FullscreenChatSettings>) => {
    const s = useAppStore.getState();
    await s.updateSettings({ ...s.settings, fullscreen_chat: { ...s.settings.fullscreen_chat, ...patch } });
    if (patch.width != null) setOverlayWidthLive(null);
    if (patch.opacity != null) setOverlayOpacityLive(null);
  }, []);
  // The handle's range is the whole screen's (the column's background runs to
  // the edge through the cutout inset; only its text keeps clear of the cutout),
  // and the video always keeps at least 240 beside it.
  const overlayMaxWidth = Math.max(240, Math.round(shape.w - 240));
  const overlayWidth = Math.min(Math.max(240, overlayWidthLive ?? fsChat?.width ?? 340), overlayMaxWidth);
  const overlayOpacity = Math.max(0, Math.min(100, overlayOpacityLive ?? fsChat?.opacity ?? 55));
  // Resizable only where there is genuinely a trade to make. On a phone the
  // 16:9 band is simply right, and there is no surplus to hand to chat.
  const resizable = shape.largeScreen && !mini && !pip;
  // The axis the divider travels along, and the natural split it starts from:
  // a 16:9 player with chat taking whatever is left.
  const axisLen = chatBeside ? shape.w : shape.h;
  const naturalPlayer = chatBeside ? shape.splitX : (shape.w * 9) / 16;
  const MIN_CHAT = chatBeside ? 260 : 150;
  const MIN_PLAYER = chatBeside ? 240 : 120;
  const desiredPlayer = tabletop
    ? (shape.splitY as number)
    : chatSplit != null
      ? axisLen * (1 - chatSplit)
      : naturalPlayer;
  const playerMain = resizable || tabletop
    ? Math.max(MIN_PLAYER, Math.min(axisLen - MIN_CHAT, desiredPlayer))
    : naturalPlayer;
  // Where the seam falls. On a Fold this is the hinge itself, so neither pane is
  // bisected by the crease; elsewhere the split the viewer chose. On a phone it
  // is the chat column's width, which the column's own handle drags live.
  const playerWidth = phoneBeside ? shape.w - overlayWidth : chatBeside ? playerMain : shape.w;

  useEffect(() => {
    const onResize = () => setViewport({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  // The clamp is applied at RENDER, not stored back into state.
  //
  // That distinction is the whole fix for the stranded mini player. `miniPos`
  // keeps the position the viewer last dragged to; `miniRect` is where the box
  // is actually allowed to be given the CURRENT viewport. Deriving instead of
  // syncing means a hostile viewport can never be persisted:
  //
  //   - A gesture-nav home swipe auto-enters PiP and resizes the activity to
  //     ~213x120 mid-gesture. Deriving clamps for that frame and then, the
  //     instant the real viewport returns, resolves back to the right place.
  //     Nothing bad is ever written down, so nothing outlives the transition -
  //     which matters because the WebView is never torn down (unconsumed back is
  //     moveTaskToBack, and PiP preserves the whole tree), so a bad stored value
  //     survives every "restart" a viewer can actually perform.
  //   - Rotation and fold posture get the same treatment for free. Previously a
  //     landscape x could leave the box entirely off the right edge in portrait
  //     with nothing left on screen to grab.
  //
  // Insets are memoized on the viewport rather than read every render:
  // getComputedStyle forces a style resolve, so the insets are read inside this
  // memo rather than every render: they can only have changed when the viewport
  // did, and the viewport is already a dependency of the clamp itself.
  // Shown higher over the Chats tab while a chat is open there, so the box
  // never covers its composer; the stored spot is kept for the other tabs.
  const overChatsComposer = useMobileNavStore((s) => s.activeTab === 'chats') && chatTabsOpen;
  // Chats has no tab bar: the box rests its usual edge gap above the composer
  // and any streak banner over it, as Chats measures them.
  const chatsFloor = useMobileNavStore((s) => s.chatsFloor);
  const miniFloor = overChatsComposer && chatsFloor != null ? chatsFloor + EDGE_GAP : BOTTOM_GAP;
  const miniClamped = useMemo(
    () =>
      clampMini(
        miniPos.x,
        miniPos.y,
        viewport.w,
        viewport.h,
        readInsets(),
        miniFloor,
      ),
    [miniPos.x, miniPos.y, viewport.w, viewport.h, miniFloor],
  );

  // A phone held upright: the layer stays laid out at full size in every mode
  // and the mini box is a translate + scale of it (miniLayer.ts), the way Xtra
  // minimizes its player. Animating the layer's real size laid out the video,
  // the controls and every chat row again on every frame, which is what made
  // shrinking and growing stutter on mid-range phones. Tablets, landscape, fold
  // postures and PiP keep the rect geometry (layerGeometry below).
  const scaleModel = !shape.largeScreen && !shape.landscape && shape.splitY == null && !pip;
  const [prevScaleModel, setPrevScaleModel] = useState(scaleModel);
  if (prevScaleModel !== scaleModel) {
    setPrevScaleModel(scaleModel);
    setModelJump(true);
  }
  const miniScale = MINI_W / viewport.w;
  // The box's own 200 px, laid out inside the full-width band and scaled back
  // up to fill it, for the few things drawn for the box itself.
  const miniFace: React.CSSProperties = {
    width: MINI_W,
    height: (MINI_W * 9) / 16,
    transform: `scale(${1 / miniScale})`,
    transformOrigin: '0 0',
  };
  // 0 is the full player, 1 the mini box. Driven by the drag and by one spring,
  // with no React render per frame.
  const prog = useMotionValue(mini ? 1 : 0);
  const boxX = useMotionValue(miniClamped.x);
  const boxY = useMotionValue(miniClamped.y);
  const fitScale = useMotionValue(miniScale);
  const bandTop = useMotionValue(0);
  const layerTransform = useTransform(() =>
    miniLayerTransform(prog.get(), fitScale.get(), boxX.get(), boxY.get(), bandTop.get()),
  );
  // The Motion setting (Interface > Motion) reaches framer's own animations
  // through MotionConfig, but not an animate() run by hand: read it here.
  const reduceMotion = useReducedMotionConfig();
  const landed = useCallback(() => settle(), [settle]);
  // While the layer is moving or small, it gets its own compositor layer so the
  // scale never redraws its contents. Only then: on the full player at rest it
  // would make the layer the containing block of chat's fixed menus.
  const inMotion = scaleModel && (mini || dragging || landing);
  // The mini box's mute corner follows the full bar's rule: only a stream
  // playing on this phone, and never a VOD.
  const vodPlayback = useAppStore((s) => s.vodPlayback);
  const mediaType = useAppStore((s) => s.currentMediaType);
  const liveMuted = useLiveVideoMuted();

  useLayoutEffect(() => {
    fitScale.set(MINI_W / viewport.w);
    bandTop.set(bandRef.current?.offsetTop ?? 0);
  }, [viewport.w, viewport.h, scaleModel, fitScale, bandTop]);

  // The box's resting corner. A drag of the box writes boxX/boxY itself (no
  // render per move) and its release animates them to the snapped spot.
  useLayoutEffect(() => {
    if (!scaleModel || !mini || reduceMotion) {
      boxX.jump(miniClamped.x);
      boxY.jump(miniClamped.y);
      return;
    }
    if (gesture.current?.kind === 'move') return;
    const ax = animate(boxX, miniClamped.x, TWEEN);
    const ay = animate(boxY, miniClamped.y, TWEEN);
    return () => {
      ax.stop();
      ay.stop();
    };
  }, [scaleModel, mini, reduceMotion, miniClamped.x, miniClamped.y, boxX, boxY]);

  // One 250 ms move for every mode change, and for the return after a drag:
  // releasing sets `dragging` false and this carries on from wherever the
  // finger left the layer.
  useLayoutEffect(() => {
    const target = mini ? 1 : 0;
    if (!scaleModel) {
      prog.jump(target);
      return;
    }
    if (dragging) return;
    // Re-read where the band sits: the safe-area inset can arrive after mount
    // without the viewport changing.
    bandTop.set(bandRef.current?.offsetTop ?? bandTop.get());
    if (reduceMotion || prog.get() === target) {
      // Nothing to animate: land on the next frame, as an animation would.
      prog.jump(target);
      const id = requestAnimationFrame(landed);
      return () => cancelAnimationFrame(id);
    }
    const a = animate(prog, target, { ...TWEEN, onComplete: landed });
    return () => a.stop();
  }, [scaleModel, reduceMotion, mini, dragging, prog, bandTop, landed]);

  // System PiP strip-down flag from MainActivity. Also mirrored onto <html> so
  // the lifecycle handler can tell real backgrounding from PiP: Android fires
  // visibilitychange=hidden for both, but in PiP the video is still on screen
  // and must keep playing.
  useEffect(() => {
    const apply = (active: boolean) => {
      setPip(active);
      document.documentElement.dataset.snPip = active ? 'true' : 'false';
    };
    const onPip = (e: Event) => apply(!!(e as CustomEvent<boolean>).detail);

    // Closing the PiP window does NOT finish the activity, so every bit of
    // state here survives into the next time the app is opened. Two things have
    // to happen on the way back in, and both are read SYNCHRONOUSLY rather than
    // trusting an event that was dispatched while the activity was stopping:
    //   1. Reconcile the flag, or a `pip` left at true renders the stripped
    //      black player full screen in an app that is not pipped at all.
    //   2. Tear the stream down if the window was dismissed rather than
    //      expanded, since otherwise it is still loaded and playing. With
    //      Background play on it is meant to be playing: the audio carried on
    //      (lockScreenAudio dropped it to the audio rendition), so it stays.
    const onVisible = () => {
      if (document.visibilityState !== 'visible') return;
      const native = isInPip();
      if (native !== null) apply(native);
      if (consumePipClosed() && !keepsAudioAfterPip()) void exitStream();
    };
    const onPipClosed = () => {
      if (!keepsAudioAfterPip()) {
        void exitStream();
        return;
      }
      // Handled: clear the native flag so the next open does not end the stream.
      consumePipClosed();
    };

    window.addEventListener('sn:pip', onPip);
    window.addEventListener('sn:pip-closed', onPipClosed);
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      window.removeEventListener('sn:pip', onPip);
      window.removeEventListener('sn:pip-closed', onPipClosed);
      document.removeEventListener('visibilitychange', onVisible);
      delete document.documentElement.dataset.snPip;
    };
  }, [exitStream]);

  // Keep-awake follows playback, not the open layer: a paused stream lets the
  // screen sleep like any other paused video, and pressing play keeps it on
  // again. Its own effect, so a pause never touches PiP or the refresh rate.
  // TV screen mode needs the screen on too: Android only shows the TV picture
  // while the app is visible, and most phones stop mirroring when it sleeps.
  const videoPaused = useLiveVideoPaused();
  // Whether anyone can see the page at all; an unseen chat pane holds still.
  const pageVisible = usePageState().visible;
  const keepAwake = (playingHere && !videoPaused) || tvOwns;
  useEffect(() => {
    holdScreenOn('player', keepAwake);
    return () => holdScreenOn('player', false);
  }, [keepAwake]);

  // Native playback affordances: PiP eligibility while playing, immersive bars
  // only for full landscape playback.
  useEffect(() => {
    // A 60 Hz window while a stream plays. The panel's 90 Hz mode was drawing
    // every frame at 90 for a picture that is 60 at most, and frame
    // presentation (RenderThread + GPU process + compositor) was the biggest
    // share of the app's CPU while watching. Restored when playback ends.
    setPreferredRefreshRate(playingHere ? 60 : 0);
    // PiP eligibility is what makes leaving the app auto-enter a floating
    // window, and it comes first whatever Background play says: that setting
    // only decides what happens when the video cannot float (the system's PiP
    // permission is off, the window was closed, the screen went off): on, the
    // stream drops to its audio rendition behind the media notification; off,
    // it pauses (lockScreenAudio). The audio-only rendition has no picture,
    // so it never floats.
    setPipEligible(playingHere && !audioOnly);
    return () => {
      setPreferredRefreshRate(0);
      setPipEligible(false);
    };
  }, [playingHere, audioOnly]);

  // Landscape is a viewing posture, never a browsing one: while the full watch
  // layer is up on a screen wider than it is tall, the status bar and the
  // navigation bar are gone whatever the arrangement (video alone, chat over
  // it, chat beside it, or a large screen's stacked layout). A swipe from the
  // edge still brings them back for a moment. The mini box is browsing and PiP
  // is the OS's own window, so both keep the bars.
  //
  // One call per flip, and the bars handed back only when the layer goes away.
  // A per-change cleanup sent "show" straight before every "hide", which is a
  // bar flash and an inset re-layout the viewer can see.
  const immersive = show && !mini && !pip && shape.landscape;
  // The rotate button holds an orientation while the full player is up;
  // leaving it (the mini box, picture in picture, closing) hands the screen
  // back to the sensor, so the rest of the app is never stuck on its side.
  const playerUp = show && !mini && !pip;
  useEffect(() => {
    if (!playerUp) setPlayerOrientation('release');
  }, [playerUp]);
  useEffect(() => {
    setImmersive(immersive);
  }, [immersive]);
  useEffect(() => () => setImmersive(false), []);

  // Tell the OS which rectangle to animate PiP from. Without this it crops and
  // scales the WHOLE activity, so the frames before React repaints show
  // whatever full-screen panel was open rather than the video.
  //
  // Measured twice: once now, once after the layer's spring settles, because
  // the geometry animates and the first read is the pre-animation rect.
  useEffect(() => {
    if (!watching) return;
    const report = () => {
      const el = bandRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      if (r.width < 1 || r.height < 1) return;
      const d = window.devicePixelRatio || 1;
      setPipSourceRect(
        Math.round(r.left * d),
        Math.round(r.top * d),
        Math.round(r.right * d),
        Math.round(r.bottom * d),
      );
    };
    report();
    const t = setTimeout(report, 420);
    return () => clearTimeout(t);
    // The saved chat width, not the live one: the column's handle moves the
    // band every frame of a drag, and one report after the save lands is enough.
  }, [watching, mini, pip, sideBySide, viewport.w, viewport.h, playerMain, chatBeside, fsChat?.width]);

  // A newly started stream always opens full (external store sync).
  useEffect(() => {
    if (channelId) setPlayerMode('full');
  }, [channelId, setPlayerMode]);

  // Pinned messages for the room on screen (shared with the Chats screen).
  const pinned = useRoomPins(watching);

  // ONE gesture on the band, serving both drags, bound UNCONDITIONALLY.
  //
  // This used to be two handler sets swapped by `mini` mid-gesture, and every
  // symptom of the "half drag glitches" bug came out of that swap: the in-flight
  // touch never got a matching pointerdown, so the mini handlers early-returned,
  // `dragging` could strand at true and kill every later animation, and a
  // synthesized click landed on a stale `moved` flag and bounced straight back
  // to full. There was also no touchcancel binding, so a gesture the OS stole
  // (edge swipe, shade pull) left the start point live and the NEXT touch was
  // measured against it.
  //
  // `kind` is decided once at pointerdown and never re-read, so a mode change
  // mid-gesture cannot reroute a drag that is already in flight.
  const onBandPointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (pip) return;
      // Mid-flight the layer is neither the full player nor the box: let the
      // spring finish rather than freeze it half way under the finger.
      const p = prog.get();
      if (scaleModel && p > 0 && p < 1) return;
      if (scaleModel && !mini) bandTop.set(bandRef.current?.offsetTop ?? bandTop.get());
      if (sideBySide) {
        // Landscape video has no layer to drag, so a vertical swipe is a dial:
        // brightness over the left half of the picture, volume over the right,
        // the way every phone video player reads those gestures. Most of the
        // band height is the whole 0..1 swing. A tap still falls through as a
        // click (controls toggle), because nothing commits until the finger
        // has moved past the slop.
        const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
        // A press in the strip along the screen's top or bottom edge, where the
        // shade and the bars are pulled (immersive mode hands those swipes to
        // us too), never dials. It is still recorded, so a tap there toggles
        // the controls and a swipe there is swallowed.
        const ins = readInsets();
        if (!dialCanStart(e.clientY, window.innerHeight, ins.gestureTop, ins.gestureBottom)) {
          gesture.current = {
            id: e.pointerId,
            kind: 'dial',
            x: e.clientX,
            y: e.clientY,
            px: 0,
            py: 0,
            lastY: e.clientY,
            lastT: e.timeStamp,
            vy: 0,
            progress: 0,
            moved: false,
          };
          suppressClick.current = false;
          return;
        }
        const axis: DialAxis = e.clientX - rect.left < rect.width / 2 ? 'brightness' : 'volume';
        const start = axis === 'brightness' ? getScreenBrightness() : getMediaVolume();
        gesture.current = {
          id: e.pointerId,
          kind: 'dial',
          x: e.clientX,
          y: e.clientY,
          px: 0,
          py: 0,
          lastY: e.clientY,
          lastT: e.timeStamp,
          vy: 0,
          progress: 0,
          moved: false,
          dial: {
            axis,
            start,
            travel: Math.max(140, rect.height * 0.8),
            lastStep: Math.round(start * 10),
            lastPushed: Math.round(start * 100),
          },
        };
        suppressClick.current = false;
        return;
      }
      gesture.current = {
        id: e.pointerId,
        kind: mini ? 'move' : 'shrink',
        x: e.clientX,
        y: e.clientY,
        // miniRect, not miniPos: the drag has to start from where the box
        // actually IS on screen. If a previous viewport left the stored position
        // out of bounds, the rendered box is the clamped one, and measuring the
        // drag from the stale origin would make it jump on the first move.
        px: miniClamped.x,
        py: miniClamped.y,
        lastY: e.clientY,
        lastT: e.timeStamp,
        vy: 0,
        progress: 0,
        moved: false,
      };
      // Only ever armed by the gesture whose click it is meant to swallow. A
      // cancelled drag produces no click, so without this the flag would sit
      // armed and eat the next real tap.
      suppressClick.current = false;
    },
    [pip, sideBySide, mini, miniClamped.x, miniClamped.y, scaleModel, prog, bandTop],
  );

  const onBandPointerMove = useCallback(
    (e: React.PointerEvent) => {
      const g = gesture.current;
      if (!g || g.id !== e.pointerId) return;
      const dx = e.clientX - g.x;
      const dy = e.clientY - g.y;
      if (!g.moved && (Math.abs(dx) > TAP_SLOP_PX || Math.abs(dy) > TAP_SLOP_PX)) {
        // A dial is a mostly vertical swipe. One that leaves the slop sideways
        // (a stray swipe across the picture, a thumb resting while the phone
        // is held), or one that began in an edge strip (the shade or bar pull
        // the OS also sees), is not a dial and not a tap: the gesture ends
        // here, touching neither brightness nor volume, and its trailing click
        // is swallowed.
        if (g.kind === 'dial' && (!g.dial || Math.abs(dy) <= Math.abs(dx))) {
          gesture.current = null;
          suppressClick.current = true;
          return;
        }
        g.moved = true;
        // Captured on first MOVE, not on pointerdown. Capturing straight away
        // would retarget pointerup to the band and rob the overlay's own
        // buttons (mute, share, minimize, quality) of their click.
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        // Track the finger 1:1 from here on. Batched with the geometry update
        // below, so the very first moved frame already skips the spring.
        setDragging(true);
        // Hold chat at the size it has now for the whole drag and its spring,
        // so the shrinking layer crops it rather than re-wrapping it per frame.
        // The scale model never resizes chat, so only the rect geometry needs it.
        if (g.kind === 'shrink' && !scaleModel && chatColumnEl) {
          const r = chatColumnEl.getBoundingClientRect();
          if (r.width > 0 && r.height > 0) setChatFreeze({ w: r.width, h: r.height });
        }
      }
      // Everything below moves the layer, so nothing happens until the gesture
      // has committed to being a drag rather than a tap.
      if (!g.moved) return;
      const dt = e.timeStamp - g.lastT;
      if (dt > 0) g.vy = (e.clientY - g.lastY) / dt;
      g.lastY = e.clientY;
      g.lastT = e.timeStamp;

      if (g.kind === 'dial' && g.dial) {
        const d = g.dial;
        const value = Math.max(0, Math.min(1, d.start - dy / d.travel));
        const hundredth = Math.round(value * 100);
        if (hundredth !== d.lastPushed) {
          d.lastPushed = hundredth;
          if (d.axis === 'brightness') {
            setScreenBrightness(value);
            brightnessTouched.current = true;
          } else {
            setMediaVolume(value);
          }
          // One light tick per tenth crossed, never per frame: a buzz that
          // tracks the finger reads as a fault, one at each notch reads as a
          // dial.
          const step = Math.round(value * 10);
          if (step !== d.lastStep) {
            d.lastStep = step;
            hapticStep();
          }
        }
        if (dialHide.current) {
          clearTimeout(dialHide.current);
          dialHide.current = null;
        }
        setDial({ axis: d.axis, value });
        return;
      }
      if (g.kind === 'move') {
        const to = clampMini(g.px + dx, g.py + dy, viewport.w, viewport.h, readInsets());
        if (scaleModel) {
          // Straight onto the transform: no render per move.
          boxX.set(to.x);
          boxY.set(to.y);
        } else {
          setMiniPos(to);
        }
        return;
      }
      // Downward and mostly vertical. Tracks back to 0 if the finger returns, so
      // an abandoned drag rewinds under the finger instead of doing nothing.
      const vertical = dy > 0 && dy > Math.abs(dx) * 1.2;
      g.progress = vertical ? Math.min(1, dy / shrinkTravel(viewport.h)) : 0;
      if (scaleModel) prog.set(g.progress);
      else setShrink(g.progress);
    },
    [viewport.w, viewport.h, chatColumnEl, scaleModel, boxX, boxY, prog],
  );

  const onBandPointerUp = useCallback(
    (e: React.PointerEvent) => {
      const g = gesture.current;
      if (!g || g.id !== e.pointerId) return;
      gesture.current = null;
      suppressClick.current = g.moved;
      setDragging(false);

      if (g.kind === 'dial') {
        // The readout lingers for a beat so the final value can be read, then
        // clears itself. A dial never moves the layer, so nothing to commit.
        if (g.moved) {
          if (dialHide.current) clearTimeout(dialHide.current);
          dialHide.current = setTimeout(() => setDial(null), 650);
        }
        return;
      }

      if (g.kind === 'move') {
        // Tap on the mini box expands. Decided here rather than in an onClick so
        // it never depends on Chromium synthesizing a compatibility click.
        // That click still comes, and it is aimed at whatever is under the
        // finger once this commit lands: the player's buttons, laid out in the
        // still-small box. Eat it.
        if (!g.moved) {
          swallowTrailingClick();
          setPlayerMode('full');
          return;
        }
        // Snap to whichever side the player ended up closest to.
        //
        // Skipped entirely while the activity is in PiP. A gesture-nav home
        // swipe auto-enters PiP and resizes the viewport MID-GESTURE, so without
        // this guard the snap below runs against a ~213x120 window and parks the
        // box off the top of the real screen. isInPip() is the synchronous
        // bridge read on purpose: the async dataset mirror loses this race.
        if (isInPip() === true) return;
        const snap = (pos: { x: number; y: number }) => {
          const ins = readInsets();
          const leftBound = MINI_GUTTER + Math.max(ins.left, ins.gestureLeft);
          const rightBound =
            viewport.w - MINI_W - MINI_GUTTER - Math.max(ins.right, ins.gestureRight);
          const snappedX = pos.x + MINI_W / 2 < viewport.w / 2 ? leftBound : rightBound;
          return clampMini(snappedX, pos.y, viewport.w, viewport.h, ins);
        };
        if (scaleModel) {
          // The drag only moved the transform; the stored spot learns the
          // release here, and the box springs to it.
          const to = snap({ x: boxX.get(), y: boxY.get() });
          setMiniPos(to);
          if (reduceMotion) {
            boxX.jump(to.x);
            boxY.jump(to.y);
          } else {
            animate(boxX, to.x, TWEEN);
            animate(boxY, to.y, TWEEN);
          }
          return;
        }
        setMiniPos(snap);
        return;
      }
      // Commit on distance OR a downward flick, always on RELEASE. In the scale
      // model the spring back (or on to the box) is the mode effect's, from
      // wherever the finger left the layer.
      if (!scaleModel) setShrink(0);
      if (g.progress >= COMMIT_FRACTION || g.vy >= FLING_VY) setPlayerMode('mini');
    },
    [viewport.w, viewport.h, setPlayerMode, scaleModel, boxX, boxY, reduceMotion],
  );

  // The OS took the gesture (edge swipe, notification shade, an incoming call).
  // Unwind without deciding anything: a cancel is not a tap and not a commit.
  // The old code bound nothing here at all, which is what left a dead start
  // point live for the NEXT touch to be measured against.
  const onBandPointerCancel = useCallback((e: React.PointerEvent) => {
    const g = gesture.current;
    if (!g || g.id !== e.pointerId) return;
    gesture.current = null;
    suppressClick.current = false;
    setDragging(false);
    setShrink(0);
    // A box drag in the scale model only moved the transform: keep the box
    // where the finger left it, as the rect geometry does.
    if (scaleModel && g.kind === 'move') setMiniPos({ x: boxX.get(), y: boxY.get() });
    if (g.kind === 'dial') {
      if (dialHide.current) clearTimeout(dialHide.current);
      dialHide.current = setTimeout(() => setDial(null), 650);
    }
  }, [scaleModel, boxX, boxY]);

  // A dialled brightness is a viewing choice, not a phone setting: it lasts
  // while a stream plays and hands the screen back to the system slider when
  // playback ends or the layer goes away.
  useEffect(() => {
    if (watching) return;
    if (brightnessTouched.current) {
      brightnessTouched.current = false;
      setScreenBrightness(-1);
    }
  }, [watching]);
  useEffect(
    () => () => {
      if (brightnessTouched.current) setScreenBrightness(-1);
      if (dialHide.current) clearTimeout(dialHide.current);
    },
    [],
  );

  // NOTE: `show` is computed near the top of the component (the mode
  // derivation needs it). There used to be a separate full-screen loading
  // return here (a giant centered logo) that was swapped wholesale for the
  // player tree the moment the URL landed — the "big logo snaps into the
  // player" jank. Now the SAME tree renders from the first loading frame: the
  // player band shows the logo at band size, chat mounts beneath, and the URL
  // arriving changes nothing structural.

  // ONE TREE FOR EVERY MODE. This used to be three separate `return`s (PiP,
  // landscape, portrait) and that was the reason rotating or entering PiP
  // reloaded the stream: React reconciles by position and type, so swapping the
  // root subtree unmounted MobilePlayer, and with it the <video> element and the
  // hls.js instance, then built a fresh one. Portrait full <-> mini was always
  // smooth for the opposite reason, and now every mode gets that property.
  //
  // The rule this render has to keep: the ancestor chain from the root down to
  // <MobilePlayer> is IDENTICAL in all four modes. Modes may change classes,
  // styles and geometry, never structure. Conditional SIBLINGS are fine.
  // Geometry of the whole layer. PiP and side-by-side fill the screen; mini is a
  // draggable box; stacked is the viewport. Mid-drag it is the INTERPOLATION
  // between full and mini, which is the whole point: the player follows the
  // finger, so a partial drag is a state you can see and back out of.
  //
  // The band is `aspect-video w-full`, so at 200px wide it resolves to 112.5px,
  // which is MINI_H. The shrink therefore reads correctly the whole way down
  // with no class juggling, and `overflow-hidden` clips chat as it goes.
  const fullRect = { top: 0, left: 0, width: viewport.w, height: viewport.h, borderRadius: 0 };
  const miniRect = {
    // miniClamped, not miniPos: the clamp is applied on the way to the screen,
    // so no viewport can leave a stored position stranding the box out of reach.
    top: miniClamped.y,
    left: miniClamped.x,
    width: MINI_W,
    height: MINI_H,
    borderRadius: 12,
  };
  // PiP is sized in PERCENTAGES, not pixels, and that is load-bearing. The OS
  // resizes the activity into a small window, so a layer pinned to
  // `viewport.w/h` is still laid out at full phone size until the `resize`
  // event lands and React re-renders - which shows a cropped corner of the app
  // inside the PiP window instead of the player. Percentages resolve against
  // the window itself and track the resize with no round trip.
  // The scale model never moves this rect: the layer stays full size and the
  // transform (layerTransform) does the shrinking.
  const layerGeometry = pip
    ? { top: 0, left: 0, width: '100%', height: '100%', borderRadius: 0 }
    : scaleModel
      ? fullRect
      : mini
      ? miniRect
      : sideBySide || shrink === 0
        ? fullRect
        : {
            top: lerp(fullRect.top, miniRect.top, shrink),
            left: lerp(fullRect.left, miniRect.left, shrink),
            width: lerp(fullRect.width, miniRect.width, shrink),
            height: lerp(fullRect.height, miniRect.height, shrink),
            borderRadius: lerp(0, miniRect.borderRadius, shrink),
          };

  // The player fills the layer except in the stacked layout, where it is a 16:9
  // band above chat.
  const playerBandClass =
    pip || (mini && !scaleModel)
      ? 'w-full h-full relative'
      : mini
        ? // The scale model's box: the full-width 16:9 band, scaled. No accent
          // rim or cast under it; the box has its own shadow.
          'w-full relative shrink-0 min-h-0 aspect-video'
      : sideBySide
        ? 'shrink-0 h-full relative'
        : // A resizable band gets an explicit height; aspect-video would fight
          // it. The video is object-contain either way, so a band shorter than
          // 16:9 pillarboxes instead of cropping or stretching.
          //
          // Stacked, video meets chat at a hard edge, and the player should
          // read as floating above it. Two deliberate choices:
          //
          // The rim is ACCENT-tinted, not a white hairline. Every divider in
          // the shell (chat tabs, settings rows, composer, pinned) already uses
          // borderSubtle, so a faint white line here just looked like one more
          // list divider in a place that is not a list. color-mix keeps it
          // tracking whichever of the themes is active.
          //
          // The cast is layered and ACCENT-TINTED rather than black. Measured on
          // device, the chat column sits at rgb(13,12,13): a black shadow over
          // that paints nothing at all. It was present in computed style and
          // completely invisible. On a near-black surface a raised edge has to
          // be described by light, not by darkening, so this is a dispersed
          // bloom in the theme's accent, widening and fading across three
          // layers.
          //
          // z-20, not z-10, and that is the whole reason this is visible. The
          // chat header (drop bar / pinned strip) is `absolute` at the top of
          // the chat column at z-10 with a 90%-opaque background and a 64px
          // backdrop blur. Tied on z-index and later in the DOM, it won, and it
          // painted over the cast: an OPAQUE RED test shadow came back as
          // rgb(26,12,13), about 5% of red. Nothing about the shadow was ever
          // weak; it was being covered by a panel that only exists while a drop
          // or a pin is showing, which is why it looked intermittent.
          // A SEPARATOR, not a wash. The wide 64px third layer was reaching far
          // enough down the chat column to tint everything under the player,
          // which is what made it read as a glow no matter how low the alpha
          // went. Two tight layers instead, dying out within ~24px, so the
          // effect is confined to the edge it is describing.
          //
          // z-10, below the chat header's z-20: the pinned card and drop bar
          // float ON the cast rather than being washed by it.
          // min-h-0 is load-bearing: without it the flex item's min-height:auto
          // lets the CONTENT overrule aspect-video. The video's 1x1 placeholder
          // poster gives it a SQUARE intrinsic ratio until metadata arrives, so
          // the loading band rendered w-full x w-full and then lurched down to
          // 16:9 on the first frame (measured on device: 361px -> 204px).
          `w-full relative shrink-0 min-h-0 z-10 border-b border-[color-mix(in_srgb,var(--color-accent)_14%,transparent)] shadow-[0_3px_8px_-2px_color-mix(in_srgb,var(--color-accent)_10%,transparent),0_8px_18px_-8px_color-mix(in_srgb,var(--color-accent)_6%,transparent)] ${
            resizable || tabletop ? '' : 'aspect-video'
          }`;

  // Chat is a column below when stacked, a side panel when side by side, and
  // hidden (but still MOUNTED) in mini and PiP. Hidden rather than unmounted so
  // folding, rotating or popping into PiP never tears down the chat connection.
  const chatClass = pip
    ? 'hidden'
    : sideBySide
      ? chatBeside
        ? 'flex-1 min-w-0 relative flex flex-col bg-background'
        : chatOverlay
          ? // Floats over the picture at the chosen edge. z-30 puts it above
            // the player's control layer, which is inset away from this edge
            // (see controlsInset below) so the two never fight for a touch.
            `absolute top-0 bottom-0 z-30 flex flex-col sn-chat-overlay ${
              overlaySide === 'left' ? 'left-0 sn-chat-overlay--left' : 'right-0'
            }`
          : 'hidden'
      : scaleModel
        ? // Never resized here. As Xtra does, it goes the moment a minimize
          // starts and is back the moment an expand starts; while the box is
          // up it skips rendering but KEEPS its layout, so the expand does not
          // lay the rows out again.
          `flex-1 min-h-0 relative flex flex-col${mini ? ' [content-visibility:hidden]' : ''}`
        : // Rect geometry: hidden while the box grows back too, then faded in
          // once the layer lands (sn-chat-reveal), so the spring never lays chat out.
          mini || landing
          ? 'hidden'
          : 'flex-1 min-h-0 relative flex flex-col sn-chat-reveal';

  return (
    <AnimatePresence>
    {show && (
    <motion.div
      key="watch-layer"
      // In system PiP the OS hands the WHOLE ACTIVITY to its window, so whatever
      // full-screen panel happened to be open paints inside it. SettingsScreen
      // and CosmeticsScreen are `z-50` and mount AFTER this layer in MobileApp,
      // so tied on z-index they won the paint order and the PiP window showed
      // settings instead of the video. This value must stay above every overlay
      // screen (z-50) and above MobileSheet's body-level portal (z-[9000]).
      //
      // The layer rises in on open and drops away on close instead of popping.
      // This wrapper carries only that and takes no touches itself: the layer
      // inside it owns the geometry and the scale model's transform, which a `y`
      // on the same element would fight.
      className={`fixed inset-0 pointer-events-none ${pip ? 'z-[9500]' : 'z-40'}`}
      initial={{ opacity: 0, y: 40 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 40, transition: { duration: 0.18, ease: 'easeIn' } }}
      transition={SPRING}
    >
    {/* glass-exempt: in the scale model this layer is clear on purpose; its
        opaque background is the faded layer just inside it. */}
    <motion.div
      className={`absolute overflow-hidden pointer-events-auto ${
        !scaleModel && (mini || shrink > 0) ? 'shadow-[0_12px_32px_-8px_rgba(0,0,0,0.65)]' : ''
      }`}
      style={{
        // The scale model paints its background on its own faded layer below,
        // so the box's surroundings are see-through as it shrinks.
        backgroundColor: scaleModel ? 'transparent' : pip || sideBySide ? '#000' : 'var(--color-background)',
        transform: scaleModel ? layerTransform : 'none',
        transformOrigin: '0 0',
        willChange: inMotion ? 'transform' : undefined,
        // Shrunk, the full-size layer would still cover Home with its invisible
        // remainder: only the band takes touches then.
        pointerEvents: scaleModel && mini ? 'none' : undefined,
      }}
      initial={false}
      animate={layerGeometry}
      transition={dragging || modelJump ? { duration: 0 } : SPRING}
      // Fires only for an animation that ran to the end (a cancelled one never
      // reports), so this is the layer genuinely at rest.
      onAnimationComplete={() => {
        if (!dragging) settle();
      }}
    >
      {/* The layer's background: gone the moment a minimize starts, so only
          the band shrinks, and back the moment an expand starts. */}
      {scaleModel && !mini && <div aria-hidden className="absolute inset-0 bg-background" />}
      <div
        // relative: the landscape chat overlay positions against this box.
        // A phone's beside column honours the overlay's side preference; the
        // reverse is a class, so the player keeps its tree position.
        className={`w-full h-full flex relative ${
          sideBySide ? (phoneBeside && overlaySide === 'left' ? 'flex-row-reverse' : 'flex-row') : 'flex-col'
        }`}
        style={{
          // Only portrait-full needs to clear the status bar; landscape draws
          // under it deliberately and mini/PiP have no bar over them. The scale
          // model keeps it in the box too: its layout never changes, and the
          // transform accounts for the inset (bandTop).
          paddingTop: (mini && !scaleModel) || pip || sideBySide ? 0 : 'var(--sn-safe-t, 0px)',
        }}
      >
        {/* The box's shadow, in the band's own pixels so it reads as the old
            box's shadow once scaled. Fixed for the whole move: nothing about it
            changes while the layer moves, so nothing is repainted. */}
        {scaleModel && mini && (
          <div
            aria-hidden
            className="absolute left-0 right-0 aspect-video pointer-events-none"
            style={{
              top: 'var(--sn-safe-t, 0px)',
              borderRadius: 12 / miniScale,
              boxShadow: `0 ${12 / miniScale}px ${32 / miniScale}px -${8 / miniScale}px rgba(0,0,0,0.65)`,
            }}
          />
        )}
        {/* The player keeps this exact tree position in EVERY mode, so the video
            element and the hls.js instance survive rotation, PiP and minimize. */}
        <motion.div
          ref={bandRef}
          className={playerBandClass}
          // Explicit width only when split: this is what puts the seam ON the
          // hinge, rather than letting flex choose a boundary the crease then
          // cuts straight through.
          //
          // touch-action is STATIC, not toggled by mode. Chromium fixes a
          // gesture's disposition at touchstart and never re-reads the property
          // mid-gesture (see the note in chat/useLongPressDrag.ts), so the old
          // `mini ? 'none' : undefined` only ever applied AFTER the drag that
          // needed it. Nothing under the band scrolls, so `none` is safe here,
          // in every mode: landscape needs it too, for the brightness and
          // volume dials.
          style={{
            // Mid-shrink the explicit resizable height is dropped, so the band
            // falls back to `aspect-video` and scales with the layer instead of
            // clipping against a fixed height. Only reachable on an expanded
            // screen in the stacked arrangement; phones never set it.
            ...(sideBySide
              ? { width: playerWidth }
              : (resizable || tabletop) && shrink === 0
                ? { height: playerMain }
                : null),
            // Scale model: the box's corners (fixed for the whole move, in the
            // band's own pixels), and in the box the band is the one thing that
            // takes touches (the layer passes the rest through to Home).
            ...(scaleModel ? { borderRadius: mini ? 12 / miniScale : 0, overflow: 'hidden' } : null),
            pointerEvents: scaleModel && mini ? 'auto' : undefined,
            touchAction: 'none',
          }}
          onPointerDown={onBandPointerDown}
          onPointerMove={onBandPointerMove}
          onPointerUp={onBandPointerUp}
          onPointerCancel={onBandPointerCancel}
          // Swallow the click that trails a drag, so releasing a gesture never
          // also toggles the player controls or expands the mini box.
          onClickCapture={(e) => {
            if (suppressClick.current) {
              suppressClick.current = false;
              e.stopPropagation();
            }
          }}
        >
          {/* Everything the band shows, in one wrapper that is there in every
              mode (the video keeps its tree position). The player is never laid
              out again for the box, it is just scaled with the layer, as Xtra
              does; only a compact cast or TV card, which has nothing to lay
              out, is drawn at the box's own 200 px (miniFace). */}
          <div
            className="absolute left-0 top-0"
            style={
              scaleModel && mini && (castOwns || tvOwns || castEnded)
                ? miniFace
                : { width: '100%', height: '100%' }
            }
          >
          {castOwns ? (
            <CastCard cast={cast} compact={mini || pip} />
          ) : tvOwns ? (
            <TvCard tv={tv} compact={mini || pip} />
          ) : castEnded ? (
            <CastEndedCard compact={mini || pip} onWatchHere={() => void restartStream()} />
          ) : (
            <MobilePlayer
              immersive={sideBySide || pip}
              compact={mini || pip}
              onMinimize={mini || pip ? undefined : () => setPlayerMode('mini')}
              // Turns the screen whatever the rotation lock says; leaving the
              // player hands it back to the sensor (the effect below).
              onRotate={
                mini || pip || shape.largeScreen
                  ? undefined
                  : () => setPlayerOrientation(shape.landscape ? 'portrait' : 'landscape')
              }
              landscapeNow={shape.landscape}
              onToggleFullscreen={
                largeToggle && !mini && !pip
                  ? () => setLargeChatHidden((v) => !v)
                  : sideBySide
                    ? () => setLandscapeChat((v) => !v)
                    : undefined
              }
              chatOpen={largeToggle ? !largeChatHidden : chatOverlay || chatBeside}
              // With chat floating over one edge, the controls keep to the
              // picture that is still visible: otherwise the chat toggle sat
              // under the very column it was meant to dismiss.
              controlsInset={chatOverlay ? { side: overlaySide, px: overlayWidth } : undefined}
              layoutMode={twoColumns ? 'columns' : 'stacked'}
              offlineCardBelow={offlineChin}
              onToggleLayout={
                // Nothing to arrange while chat is hidden.
                canChooseLayout && !mini && !pip && !(largeToggle && largeChatHidden)
                  ? () => {
                      // Writes an explicit choice, so it stops tracking `auto` and
                      // survives a restart.
                      const next: WatchLayout = twoColumns ? 'stacked' : 'columns';
                      setWatchLayout(next);
                      writeWatchLayout(next);
                    }
                  : undefined
              }
            />
          )}
          </div>
          {/* IN-APP mini only. In system PiP the OS draws its own close and
              expand buttons over the window, so rendering ours too gave two
              exit buttons; and taps inside a PiP window go to the OS chrome,
              never to this content, so the swallower has nothing to catch. PiP
              carries no web chrome at all - mute is a native RemoteAction.
              In the scale model the band is the full-width band scaled down, so
              these few controls are laid out at the box's own 200 px and scaled
              back up: the box's corners look exactly as designed, and the
              player itself is never laid out again. */}
          {mini && !pip && (
            <div
              className="absolute left-0 top-0 z-10"
              style={scaleModel ? miniFace : { width: '100%', height: '100%' }}
            >
              {/* Swallow player taps so the whole box reads as one control. */}
              <div className="absolute inset-0 z-10" />
              {/* Two corner controls, the ENTIRE corner each: the top-right
                  closes, the bottom-left mutes; a tap anywhere else expands. A
                  corner-hugging circle on a small floating box fails twice
                  over: half the finger pad spills outside the box (the layer
                  clips, so those taps hit whatever is underneath), and the rest
                  competes with the expand surface. 56px is the box's full
                  height-half, and the 32px circle just marks where it lives. */}
              <button
                className="absolute top-0 right-0 z-20 w-14 h-14 flex items-start justify-end p-1.5"
                aria-label="Close stream"
                onPointerDown={(e) => {
                  // A new tap clears any stale trailing-click suppression. The
                  // band normally does this in ITS pointerdown, but this button
                  // stops propagation (so a tap here never starts a drag) —
                  // making it the one control that could not reset the flag.
                  // On touch, the swipe that CREATES the mini box arms the
                  // suppressor and produces no trailing click to consume it,
                  // so the very next X tap was always swallowed: the reported
                  // "first tap focuses, second tap closes".
                  suppressClick.current = false;
                  e.stopPropagation();
                }}
                onClick={(e) => {
                  e.stopPropagation();
                  void exitStream();
                }}
              >
                <span className="w-8 h-8 rounded-full bg-black/60 flex items-center justify-center text-white">
                  <X size={16} weight="bold" />
                </span>
              </button>
              {/* Mute under the full bar's rule: a stream playing on this
                  phone, never a VOD. The same toggle as the bar and the PiP
                  window's action, so all three agree. */}
              {playingHere && !(vodPlayback && mediaType !== 'live') && (
                <button
                  className="absolute bottom-0 left-0 z-20 w-14 h-14 flex items-end justify-start p-1.5"
                  aria-label={liveMuted ? 'Unmute' : 'Mute'}
                  onPointerDown={(e) => {
                    // As the X: a press here never starts a drag, and clears a
                    // stale trailing-click suppression.
                    suppressClick.current = false;
                    e.stopPropagation();
                  }}
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleLiveMute();
                  }}
                >
                  <span className="w-8 h-8 rounded-full bg-black/60 flex items-center justify-center text-white">
                    {liveMuted ? <SpeakerSlash size={16} weight="bold" /> : <SpeakerHigh size={16} weight="bold" />}
                  </span>
                </button>
              )}
            </div>
          )}
          <DialIndicator dial={dial} />
          {/* The box's hairline edge, so it reads over any thumbnail under it.
              In the band's own pixels, so it is 1 px on screen in the box. */}
          {scaleModel && mini && (
            <div
              aria-hidden
              className="absolute inset-0 z-[35] pointer-events-none"
              style={{
                borderRadius: 12 / miniScale,
                boxShadow: `inset 0 0 0 ${1 / miniScale}px rgba(255, 255, 255, 0.12)`,
              }}
            />
          )}
        </motion.div>

        {/* The offline room's card sits under the band, never over the art. */}
        {offlineChin && <MobileOfflineChin />}

        {/* Between the panes, so its parent rect is the container the drag
            measures against. Hidden in mini/PiP and on phones. */}
        {resizable && (sideBySide ? chatBeside : true) && (
          <SplitHandle
            axis={chatBeside ? 'x' : 'y'}
            length={axisLen}
            minChat={MIN_CHAT}
            minPlayer={MIN_PLAYER}
            // Only meaningful across a vertical hinge, and only when the panes
            // are side by side; a horizontal divider cannot land on it.
            snapAt={chatBeside ? (shape.fold?.vertical ? shape.splitX : null) : shape.splitY}
            onDrag={(frac) => {
              setChatSplit(frac);
              writeWatchSplit(frac);
            }}
            onReset={() => {
              setChatSplit(null);
              writeWatchSplit(null);
            }}
          />
        )}

        <motion.div
          ref={setChatColumnEl}
          className={chatClass}
          style={
            // Side by side, the row deliberately carries no top padding so the
            // video runs edge to edge under the status bar. Chat is not video:
            // without its own inset the first message renders behind the clock.
            phoneBeside
              ? // Immersive, so no status bar to clear; only a camera cutout
                // on the column's outer edge.
                {
                  [overlaySide === 'left' ? 'paddingLeft' : 'paddingRight']:
                    `var(--sn-safe-${overlaySide === 'left' ? 'l' : 'r'}, 0px)`,
                }
              : sideBySide && chatBeside
              ? { paddingTop: 'var(--sn-safe-t, 0px)' }
              : chatOverlay
                ? ({
                    width: overlayWidth,
                    '--fs-chat-opacity': `${overlayOpacity}%`,
                    // The bars are hidden in landscape but a camera cutout is
                    // not; keep the column's edge off it.
                    [overlaySide === 'left' ? 'paddingLeft' : 'paddingRight']:
                      `var(--sn-safe-${overlaySide === 'left' ? 'l' : 'r'}, 0px)`,
                  } as React.CSSProperties)
                : scaleModel
                  ? undefined
                  : chatFreeze
                    ? // A shrink drag: chat keeps its full-size box and the layer's
                      // overflow crops it (see chatFreeze).
                      { width: chatFreeze.w, height: chatFreeze.h, flex: 'none' }
                    : undefined
          }
        >
          {/* Chat header carries only live, transient signal now: the hype
              train and the pinned message. The persistent stream info lives in
              the player overlay, so chat keeps its full height. The poll,
              prediction and drop cards are not in here: they are the stack in
              the chat pane's flow below, and this header starts under it, so
              nothing paints over a card's fold control. */}
            <div
              className={
                // The floating landscape column is for reading the room, so
                // the stream's own banners (pins, hype train) stay out of it;
                // they are all still one rotation away.
                !chatOverlay && ((streamScoped && currentHypeTrain) || pinned.length > 0 || showMentionCard)
                  ? // Pure layout now: no background, no border, no blur, no
                    // shadow. Each child already carries its own container
                    // (PinnedBanner is an sn-popover, the hype train draws its
                    // own filled bar), so wrapping them in a full-width panel
                    // put a box inside a box and killed any sense of the pinned
                    // message floating over chat.
                    //
                    // z-20 keeps these cards ABOVE the player's cast, so the
                    // glow passes behind them instead of washing across them.
                    'absolute left-0 right-0 px-2.5 pt-1.5 pb-2 z-20 pointer-events-none flex flex-col-reverse gap-1'
                  : 'hidden'
              }
              style={{
                // Sits BELOW the chat tab strip when several rooms are open,
                // and below the card stack under it. This header is absolutely
                // positioned over the top of the chat column, so at top:0 it
                // covered the tabs completely.
                top: topStackBottom ?? (chatTabsVisible ? CHAT_TAB_STRIP_H : 0),
              }}
            >
              {/* The hype train belongs to the STREAM, not to whichever chat
                  tab you are reading. Showing it over another room's chat
                  credits it to the wrong channel. */}
              {streamScoped && currentHypeTrain && (
                <HypeTrainBanner
                  train={currentHypeTrain}
                  onExpire={() => useAppStore.getState().setCurrentHypeTrain(null)}
                />
              )}
              <PinnedBanner
                pins={pinned}
                channelId={chatChannelId ?? undefined}
                expanded={pins.open}
                onToggle={() => pins.setOpen(!pins.open)}
                collapsedStyle={pins.style}
              />
              {/* Last in DOM, so first on screen: the mention you just opened. */}
              {showMentionCard && mentionCard && <MentionContextCard entry={mentionCard} />}
            </div>

          <MobileChatPane
            readOnly={chatOverlay}
            // Hidden under the mini player, in PiP or a chat-less landscape, or
            // the app in the background: the list holds still until seen.
            idle={chatClass === 'hidden' || !pageVisible}
            top={
              // Poll + prediction cards are exactly the desktop components:
              // they self-fetch off the channel. Either can be turned off.
              // Rust orders the stack (the user's order, votes before the
              // drop), keeps one card open and caps its height. Stream scoped,
              // like the hype train; the drop card stays mounted on another tab
              // so its poll keeps running, it just draws nothing there.
              <ChatOverlayStack
                maxHeight={topStack.maxHeight}
                stackRef={setTopStackEl}
                className="px-2.5 pt-1.5 pb-1 transition-[padding] duration-200 empty:py-0"
              >
                {currentStream && streamScoped && !chatOverlay && showPredictions && (
                  <PredictionOverlay
                    channelId={currentStream.user_id}
                    channelLogin={currentStream.user_login}
                    stack={topStack.control.prediction}
                  />
                )}
                {currentStream && streamScoped && !chatOverlay && showPolls && (
                  <PollOverlay
                    channelId={currentStream.user_id}
                    channelLogin={currentStream.user_login}
                    stack={topStack.control.poll}
                  />
                )}
                <DropProgressBar stack={topStack.control.drop} visible={streamScoped && !chatOverlay} />
              </ChatOverlayStack>
            }
          />
          {chatOverlay && (
            <ChatOverlayChrome
              side={overlaySide}
              width={overlayWidth}
              minWidth={240}
              maxWidth={overlayMaxWidth}
              onWidthChange={setOverlayWidthLive}
              onWidthCommit={(w) => void writeOverlay({ width: w })}
              strip={{
                opacity: overlayOpacity,
                onOpacityChange: setOverlayOpacityLive,
                onOpacityCommit: (o) => void writeOverlay({ opacity: o }),
                onHide: () => setLandscapeChat(false),
              }}
            />
          )}
          {/* Beside the picture there is nothing to see through, so only the
              width handle: dragging it hands the picture the difference. */}
          {phoneBeside && (
            <ChatOverlayChrome
              side={overlaySide}
              width={overlayWidth}
              minWidth={240}
              maxWidth={overlayMaxWidth}
              onWidthChange={setOverlayWidthLive}
              onWidthCommit={(w) => void writeOverlay({ width: w })}
            />
          )}
        </motion.div>
      </div>

      {/* A hairline edge on the floating box, so it reads over any thumbnail
          under it; on a dark card the shadow alone does not. It fades in with
          the shrink, so the full-screen player shows none. Above the content,
          and never in the way of a touch. Rect geometry only: the scale model
          draws its own inside the band. */}
      {!scaleModel && (mini || shrink > 0) && !pip && (
        <div
          aria-hidden
          className="absolute inset-0 z-[35] pointer-events-none"
          style={{
            borderRadius: 'inherit',
            boxShadow: `inset 0 0 0 1px rgba(255, 255, 255, ${(0.12 * (mini ? 1 : shrink)).toFixed(3)})`,
            transition: dragging ? undefined : 'box-shadow 200ms ease-out',
          }}
        />
      )}
      {/* While the box grows back to full, one surface takes every press, so
          nothing in the moving layer (the player's buttons, chat) is pressed
          mid-flight. It carries its own click handler on purpose: the WebView
          nudges a tap onto the nearest pressable element, and inside the layer
          this is always the nearest, never a card beside the box. */}
      {landing && (
        <div
          aria-hidden
          role="presentation"
          className="absolute inset-0 z-[36]"
          onClick={(e) => e.stopPropagation()}
        />
      )}
    </motion.div>
    </motion.div>
    )}
    </AnimatePresence>
  );
};
