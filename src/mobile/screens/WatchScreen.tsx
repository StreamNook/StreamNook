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
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { X } from 'phosphor-react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { usePinStore } from '../../stores/pinStore';
import { useWindowShape } from '../ui/useWindowShape';
import {
  readWatchLayout,
  readWatchSplit,
  writeWatchLayout,
  writeWatchSplit,
  type WatchLayout,
} from '../watch/watchLayout';
import { SplitHandle } from '../watch/SplitHandle';
import { MobilePlayer } from '../player/MobilePlayer';
import { MobileChatPane } from '../chat/MobileChatPane';
import {
  CHAT_TAB_STRIP_H,
  useActiveChatChannelId,
  useChatTabsVisible,
  useViewingStreamChat,
} from '../chat/ChatTabStrip';
import { DropProgressBar } from '../watch/DropProgressBar';
import { PinnedBanner } from '../watch/PinnedBanner';
import HypeTrainBanner from '../../components/HypeTrainBanner';
import PollOverlay from '../../components/PollOverlay';
import PredictionOverlay from '../../components/PredictionOverlay';
import {
  consumePipClosed,
  isInPip,
  setImmersive,
  setKeepScreenOn,
  setPipEligible,
  setPipSourceRect,
} from '../nativeBridge';
import { readInsets, type ResolvedInsets } from '../nativeInsets';
import { Logger } from '../../utils/logger';
import { isBackgrounded } from '../backgroundGate';

interface PinnedMessage {
  id: string;
  message_id: string;
  message_text: string;
  sender_name: string;
  sender_color: string;
}

const MINI_W = 200;
const MINI_H = Math.round((MINI_W * 9) / 16);
const EDGE_GAP = 12;
// Clearance so the resting mini player never sits under the floating pill bar.
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
): { x: number; y: number } {
  // Clear of the BACK strips at the sides, and of the status bar plus the home /
  // quick-switch strips top and bottom. An app cannot opt out of the home
  // gesture at all, so simply not overlapping it is the only thing that works.
  const left = EDGE_GAP + Math.max(ins.left, ins.gestureLeft);
  const right = EDGE_GAP + Math.max(ins.right, ins.gestureRight);
  const top = EDGE_GAP + Math.max(ins.top, ins.gestureTop);
  const bottom = Math.max(BOTTOM_GAP, ins.bottom + ins.gestureBottom + EDGE_GAP);

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

const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

export const WatchScreen: React.FC = () => {
  const isLoading = useAppStore((s) => s.isLoading);
  const streamUrl = useAppStore((s) => s.streamUrl);
  const currentStream = useAppStore((s) => s.currentStream);
  const currentHypeTrain = useAppStore((s) => s.currentHypeTrain);
  const exitStream = useAppStore((s) => s.exitStream);
  const playerMode = useMobileNavStore((s) => s.playerMode);
  const setPlayerMode = useMobileNavStore((s) => s.setPlayerMode);
  const refreshNonce = usePinStore((s) => s.refreshNonce);
  const shape = useWindowShape();
  const [landscapeChat, setLandscapeChat] = useState(false);
  const [watchLayout, setWatchLayout] = useState<WatchLayout>(readWatchLayout);
  const [chatSplit, setChatSplit] = useState<number | null>(readWatchSplit);
  const [pip, setPip] = useState(false);
  const [pinnedFor, setPinnedFor] = useState<{
    channel: string | null;
    items: PinnedMessage[];
  }>({ channel: null, items: [] });
  const [pinsOpen, setPinsOpen] = useState(false);
  const [dropActive, setDropActive] = useState(false);
  const chatTabsVisible = useChatTabsVisible();
  const viewingStreamChat = useViewingStreamChat();
  const chatChannelId = useActiveChatChannelId();
  // Only surface pins that belong to the room currently on screen.
  const pinned = pinnedFor.channel === chatChannelId ? pinnedFor.items : [];
  const backgroundMode = useAppStore((s) => s.settings.video_player?.background_mode) ?? 'pip';
  const [viewport, setViewport] = useState(() => ({
    w: window.innerWidth,
    h: window.innerHeight,
  }));
  // Through the clamp even on first mount, so the default resting spot respects
  // the gesture strips on a device whose insets differ from the hardcoded gaps.
  const [miniPos, setMiniPos] = useState(() =>
    clampMini(
      window.innerWidth - MINI_W - EDGE_GAP,
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
    kind: 'shrink' | 'move';
    x: number;
    y: number;
    px: number;
    py: number;
    lastY: number;
    lastT: number;
    vy: number;
    progress: number;
    moved: boolean;
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
  if (show && !prevShow) {
    setPrevShow(true);
    setForceFull(true);
  } else if (!show && prevShow) {
    setPrevShow(false);
    setForceFull(false);
  } else if (forceFull && playerMode === 'full') {
    setForceFull(false);
  }
  useEffect(() => {
    if (forceFull) setPlayerMode('full');
  }, [forceFull, setPlayerMode]);

  const mini = !forceFull && playerMode === 'mini';
  const watching = !!streamUrl && streamUrl !== 'offline';
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
  const canChooseLayout = shape.sizeClass === 'expanded' && sideFits && stackFits;
  // Only `expanded` re-decides. A phone in landscape is after immersive video,
  // so chat stays opt-in behind the fullscreen toggle there, as before.
  const twoColumns =
    shape.sizeClass === 'expanded'
      ? watchLayout === 'auto'
        ? autoColumns
        : watchLayout === 'columns' && sideFits
      : landscapeChat;
  const immersiveLandscape = shape.twoPane && shape.sizeClass !== 'expanded';
  const sideBySide = shape.twoPane && !mini && !pip && (twoColumns || immersiveLandscape);
  const chatBeside = sideBySide && twoColumns;
  // Resizable only where there is genuinely a trade to make. On a phone the
  // 16:9 band is simply right, and there is no surplus to hand to chat.
  const resizable = shape.sizeClass === 'expanded' && !mini && !pip;
  // The axis the divider travels along, and the natural split it starts from:
  // a 16:9 player with chat taking whatever is left.
  const axisLen = chatBeside ? shape.w : shape.h;
  const naturalPlayer = chatBeside ? shape.splitX : (shape.w * 9) / 16;
  const MIN_CHAT = chatBeside ? 260 : 150;
  const MIN_PLAYER = chatBeside ? 240 : 120;
  const desiredPlayer = chatSplit != null ? axisLen * (1 - chatSplit) : naturalPlayer;
  const playerMain = resizable
    ? Math.max(MIN_PLAYER, Math.min(axisLen - MIN_CHAT, desiredPlayer))
    : naturalPlayer;
  // Where the seam falls. On a Fold this is the hinge itself, so neither pane is
  // bisected by the crease; elsewhere the split the viewer chose.
  const playerWidth = chatBeside ? playerMain : shape.w;

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
  const miniClamped = useMemo(
    () => clampMini(miniPos.x, miniPos.y, viewport.w, viewport.h, readInsets()),
    [miniPos.x, miniPos.y, viewport.w, viewport.h],
  );

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
    //      expanded, since otherwise it is still loaded and playing.
    const onVisible = () => {
      if (document.visibilityState !== 'visible') return;
      const native = isInPip();
      if (native !== null) apply(native);
      if (consumePipClosed()) void exitStream();
    };
    const onPipClosed = () => void exitStream();

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

  // Native playback affordances: stay awake + PiP eligibility while playing,
  // immersive bars only for full landscape playback.
  useEffect(() => {
    setKeepScreenOn(watching);
    // PiP eligibility is what makes leaving the app auto-enter a floating
    // window. Gated on the setting, because with it always on there was no way
    // to simply MINIMISE a stream: every exit became PiP. In 'audio' mode the
    // app just goes to the background, the stream drops to its audio-only
    // rendition, and the media notification is what brings it back.
    setPipEligible(watching && backgroundMode !== 'audio');
    return () => {
      setKeepScreenOn(false);
      setPipEligible(false);
    };
  }, [watching, backgroundMode]);

  useEffect(() => {
    // Hide the system bars only when video actually fills the screen. With chat
    // beside it on a tablet or an unfolded Fold the bars are wanted.
    setImmersive(watching && sideBySide && !chatBeside);
    return () => setImmersive(false);
  }, [watching, sideBySide, chatBeside]);

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
  }, [watching, mini, pip, sideBySide, viewport.w, viewport.h, playerMain]);

  // A newly started stream always opens full (external store sync).
  useEffect(() => {
    if (channelId) setPlayerMode('full');
  }, [channelId, setPlayerMode]);

  // Pinned messages: 30s poll + instant refresh on pin/unpin actions.
  //
  // Keyed to the CHAT on screen, not to the stream. A pinned message is a
  // property of the room you are reading, so with several tabs open, fetching
  // against `currentStream` left one channel's pin sitting above another
  // channel's messages and appearing to jump between them as you switched.
  //
  // The result is stamped with the channel it describes and compared on read, so
  // a slow response cannot land under the wrong room and nothing has to clear
  // state from inside the effect.
  useEffect(() => {
    if (!chatChannelId || !watching) return;
    let cancelled = false;
    const fetchPins = async () => {
      try {
        const messages = await invoke<PinnedMessage[]>('get_pinned_chat_messages', {
          channelId: chatChannelId,
        });
        if (cancelled) return;
        setPinnedFor({ channel: chatChannelId, items: messages || [] });
        usePinStore
          .getState()
          .setPinnedIds((messages || []).map((m) => m.message_id).filter(Boolean));
      } catch (err) {
        Logger.warn('[Watch] pinned fetch failed:', err);
      }
    };
    void fetchPins();
    // Skipped while backgrounded: a pinned message is chat chrome nobody can
    // see, and the next tick after resuming picks up whatever is current.
    const t = setInterval(() => {
      if (isBackgrounded()) return;
      void fetchPins();
    }, 30_000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, [chatChannelId, watching, refreshNonce]);

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
      if (pip || sideBySide) return;
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
    [pip, sideBySide, mini, miniClamped.x, miniClamped.y],
  );

  const onBandPointerMove = useCallback(
    (e: React.PointerEvent) => {
      const g = gesture.current;
      if (!g || g.id !== e.pointerId) return;
      const dx = e.clientX - g.x;
      const dy = e.clientY - g.y;
      if (!g.moved && (Math.abs(dx) > TAP_SLOP_PX || Math.abs(dy) > TAP_SLOP_PX)) {
        g.moved = true;
        // Captured on first MOVE, not on pointerdown. Capturing straight away
        // would retarget pointerup to the band and rob the overlay's own
        // buttons (mute, share, minimize, quality) of their click.
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        // Track the finger 1:1 from here on. Batched with the geometry update
        // below, so the very first moved frame already skips the spring.
        setDragging(true);
      }
      // Everything below moves the layer, so nothing happens until the gesture
      // has committed to being a drag rather than a tap.
      if (!g.moved) return;
      const dt = e.timeStamp - g.lastT;
      if (dt > 0) g.vy = (e.clientY - g.lastY) / dt;
      g.lastY = e.clientY;
      g.lastT = e.timeStamp;

      if (g.kind === 'move') {
        setMiniPos(clampMini(g.px + dx, g.py + dy, viewport.w, viewport.h, readInsets()));
        return;
      }
      // Downward and mostly vertical. Tracks back to 0 if the finger returns, so
      // an abandoned drag rewinds under the finger instead of doing nothing.
      const vertical = dy > 0 && dy > Math.abs(dx) * 1.2;
      g.progress = vertical ? Math.min(1, dy / shrinkTravel(viewport.h)) : 0;
      setShrink(g.progress);
    },
    [viewport.w, viewport.h],
  );

  const onBandPointerUp = useCallback(
    (e: React.PointerEvent) => {
      const g = gesture.current;
      if (!g || g.id !== e.pointerId) return;
      gesture.current = null;
      suppressClick.current = g.moved;
      setDragging(false);

      if (g.kind === 'move') {
        // Tap on the mini box expands. Decided here rather than in an onClick so
        // it never depends on Chromium synthesizing a compatibility click.
        if (!g.moved) {
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
        setMiniPos((pos) => {
          const ins = readInsets();
          const leftBound = EDGE_GAP + Math.max(ins.left, ins.gestureLeft);
          const rightBound =
            viewport.w - MINI_W - EDGE_GAP - Math.max(ins.right, ins.gestureRight);
          const snappedX = pos.x + MINI_W / 2 < viewport.w / 2 ? leftBound : rightBound;
          return clampMini(snappedX, pos.y, viewport.w, viewport.h, ins);
        });
        return;
      }
      // Commit on distance OR a downward flick, always on RELEASE.
      setShrink(0);
      if (g.progress >= COMMIT_FRACTION || g.vy >= FLING_VY) setPlayerMode('mini');
    },
    [viewport.w, viewport.h, setPlayerMode],
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
  }, []);

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
  const layerGeometry = pip
    ? { top: 0, left: 0, width: '100%', height: '100%', borderRadius: 0 }
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
    pip || mini
      ? 'w-full h-full relative'
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
            resizable ? '' : 'aspect-video'
          }`;

  // Chat is a column below when stacked, a side panel when side by side, and
  // hidden (but still MOUNTED) in mini and PiP. Hidden rather than unmounted so
  // folding, rotating or popping into PiP never tears down the chat connection.
  const chatClass = pip
    ? 'hidden'
    : sideBySide
      ? chatBeside
        ? 'flex-1 min-w-0 relative flex flex-col bg-background'
        : 'hidden'
      : mini
        ? 'hidden'
        : 'flex-1 min-h-0 relative flex flex-col';

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
      className={`fixed overflow-hidden ${pip ? 'z-[9500]' : 'z-40'} ${
        mini || shrink > 0 ? 'shadow-[0_12px_32px_-8px_rgba(0,0,0,0.65)]' : ''
      }`}
      style={{
        backgroundColor: pip || sideBySide ? '#000' : 'var(--color-background)',
      }}
      // The layer rises in on open and drops away on close instead of popping.
      // `y` is a transform, so it composes with the top/left geometry without
      // disturbing layout or the drag interpolation; the spread seeds the
      // geometry at its current-mode rect so a stream opened while the previous
      // one sat in mini still enters at the right place and size.
      initial={{ ...layerGeometry, opacity: 0, y: 40 }}
      animate={{ ...layerGeometry, opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 40, transition: { duration: 0.18, ease: 'easeIn' } }}
      transition={dragging ? { duration: 0 } : SPRING}
    >
      <div
        className={`w-full h-full flex ${sideBySide ? 'flex-row' : 'flex-col'}`}
        style={{
          // Only portrait-full needs to clear the status bar; landscape draws
          // under it deliberately and mini/PiP have no bar over them.
          paddingTop: mini || pip || sideBySide ? 0 : 'var(--sn-safe-t, 0px)',
        }}
      >
        {/* The player keeps this exact tree position in EVERY mode, so the video
            element and the hls.js instance survive rotation, PiP and minimize. */}
        <div
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
          // needed it. Nothing under the band scrolls, so `none` is safe here.
          style={{
            // Mid-shrink the explicit resizable height is dropped, so the band
            // falls back to `aspect-video` and scales with the layer instead of
            // clipping against a fixed height. Only reachable on an expanded
            // screen in the stacked arrangement; phones never set it.
            ...(sideBySide
              ? { width: playerWidth }
              : resizable && shrink === 0
                ? { height: playerMain }
                : null),
            touchAction: sideBySide ? undefined : 'none',
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
          <MobilePlayer
            immersive={sideBySide || pip}
            compact={mini || pip}
            onMinimize={mini || pip ? undefined : () => setPlayerMode('mini')}
            onToggleFullscreen={sideBySide ? () => setLandscapeChat((v) => !v) : undefined}
            layoutMode={twoColumns ? 'columns' : 'stacked'}
            onToggleLayout={
              canChooseLayout && !mini && !pip
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
          {/* IN-APP mini only. In system PiP the OS draws its own close and
              expand buttons over the window, so rendering ours too gave two
              exit buttons; and taps inside a PiP window go to the OS chrome,
              never to this content, so the swallower has nothing to catch. PiP
              carries no web chrome at all - mute is a native RemoteAction. */}
          {mini && !pip && (
            <>
              {/* Swallow player taps so the whole box reads as one control. */}
              <div className="absolute inset-0 z-10" />
              {/* The ENTIRE top-right quadrant of the mini box closes. A
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
            </>
          )}
        </div>

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
            snapAt={chatBeside && shape.fold?.vertical ? shape.splitX : null}
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

        <div
          className={chatClass}
          style={
            // Side by side, the row deliberately carries no top padding so the
            // video runs edge to edge under the status bar. Chat is not video:
            // without its own inset the first message renders behind the clock.
            sideBySide && chatBeside ? { paddingTop: 'var(--sn-safe-t, 0px)' } : undefined
          }
        >
          {/* Chat header carries only live, transient signal now: the hype
              train and the pinned message. The persistent stream info lives in
              the player overlay, so chat keeps its full height. */}
          {/* Always mounted, hidden when empty. DropProgressBar has to run to
              discover whether there is a drop to show, and gating the wrapper on
              its answer would mean it never mounts to give one. `hidden` still
              mounts children and runs their effects, so this breaks that cycle
              without painting an empty strip. */}
            <div
              className={
                (viewingStreamChat && (currentHypeTrain || dropActive)) || pinned.length > 0
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
                // Sits BELOW the chat tab strip when several rooms are open.
                // This header is absolutely positioned over the top of the chat
                // column, so at top:0 it covered the tabs completely.
                top: chatTabsVisible ? CHAT_TAB_STRIP_H : 0,
              }}
            >
              {/* Hype train and drop progress belong to the STREAM, not to
                  whichever chat tab you are reading. Showing them over another
                  room's chat credits them to the wrong channel. */}
              {viewingStreamChat && currentHypeTrain && (
                <HypeTrainBanner
                  train={currentHypeTrain}
                  onExpire={() => useAppStore.getState().setCurrentHypeTrain(null)}
                />
              )}
              {/* flex-col-reverse means DOM order here renders ABOVE the pinned
                  strip and below the info row. Kept mounted even on another tab
                  so its poll keeps running; it just does not render there. */}
              <DropProgressBar
                onActiveChange={setDropActive}
                visible={viewingStreamChat}
              />
              <PinnedBanner
                pins={pinned}
                expanded={pinsOpen}
                onToggle={() => setPinsOpen((v) => !v)}
              />
            </div>

          {/* Poll + prediction cards, exactly the desktop components: they
              self-fetch off the channel and anchor under the header. Stream
              scoped, so same rule as the hype train above. */}
          {currentStream && viewingStreamChat && (
            <>
              <PredictionOverlay
                channelId={currentStream.user_id}
                channelLogin={currentStream.user_login}
              />
              <PollOverlay
                channelId={currentStream.user_id}
                channelLogin={currentStream.user_login}
              />
            </>
          )}

          <MobileChatPane />
        </div>
      </div>

    </motion.div>
    )}
    </AnimatePresence>
  );
};
