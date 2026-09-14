// Resolves playable YouTube stream URLs inside an isolated webview.
//
// The problem this solves: YouTube's live HLS ladder stops at 1080p, and the
// higher renditions exist only as per-itag `adaptiveFormats` URLs that a plain
// HTTP client cannot use. Three things stand in the way, and all three need a
// browser:
//
//   1. `n`, a throttling nonce that must be descrambled by a function generated
//      into the player's base.js. Measured: the page ships `n=szGMzhT72lLdpvqg`
//      and the player actually requests `n=QysJjwoaRjaEVw`, from a different
//      host. Sending the raw value is refused.
//   2. A Proof-of-Origin token, produced by BotGuard, an obfuscated integrity
//      check that needs real browser APIs.
//   3. `alr=yes`, which makes the server answer with a redirect body instead of
//      media.
//
// So the work happens here, in a hidden WebView2 window, and Rust receives
// finished URLs it can fetch `&sq=N` fragments from. This is the same approach
// Invidious takes (`invidious-companion`, `youtubePlayerHandling.ts`): decipher
// the adaptive formats, force `alr=no`, and carry a PO token. Notably Invidious
// does NOT use SABR, and neither do we.
//
// Deliberately isolated: this runs in a window with no Tauri API surface and
// reports back only through the URL fragment. BotGuard's interpreter is
// third-party code fetched at runtime and must never run where it could reach
// the app's IPC.
import { Innertube, Platform, UniversalCache } from 'youtubei.js/web';
import { BotGuardClient, getChallenge } from 'bgutils-js/botguard';
import { WebPoMinter } from 'bgutils-js/webpo';
import { buildURL, GOOG_API_KEY } from 'bgutils-js/utils';

const WAA_REQUEST_KEY = 'O43z0dpjhgX20SCx4KAo';

/// `fetch` must be called with `window` as its receiver. Handing the bare
/// reference to a library that stores and later calls it detaches that binding,
/// which surfaces as "Illegal invocation" from somewhere deep inside the
/// library rather than at the point of the mistake.
const boundFetch = (...args) => fetch(...args);

/**
 * A Trusted Types policy that passes strings through, created once.
 *
 * `require-trusted-types-for 'script'` covers eval and the Function constructor,
 * not just DOM sinks, so string-to-code needs a policy here. `default` is tried
 * first because it applies implicitly everywhere; YouTube may already own that
 * name, in which case a private one still works for explicit conversions.
 */
let scriptPolicy;
let scriptPolicyTried = false;
function getScriptPolicy() {
  if (scriptPolicyTried) return scriptPolicy;
  scriptPolicyTried = true;
  const tt = globalThis.trustedTypes;
  if (tt && typeof tt.createPolicy === 'function') {
    for (const name of ['default', 'streamnook-ytjs']) {
      try {
        scriptPolicy = tt.createPolicy(name, { createScript: (s) => s });
        return scriptPolicy;
      } catch {
        /* name taken or disallowed; try the next */
      }
    }
  }
  return scriptPolicy;
}

/** Indirect eval, so the snippet runs in global scope, via a policy if required. */
function runScript(source) {
  const policy = getScriptPolicy();
  const payload = policy ? policy.createScript(source) : source;
  // eslint-disable-next-line no-eval
  return (0, eval)(payload);
}

/**
 * youtubei.js 18 ships no JavaScript evaluator and refuses to decipher without
 * one, because deciphering means running a snippet extracted from YouTube's
 * `base.js` and most host environments should not do that implicitly.
 *
 * The contract, from `PlatformShim`: run `data.output` with `env` bound as
 * variables, then hand back the values named in `data.exported`.
 *
 * The env is handed over through a global rather than as function parameters,
 * because building a parameterised function would mean `new Function`, which is
 * a Trusted Types sink of its own. One policy-wrapped eval is simpler than two
 * sinks to satisfy.
 */
function evaluateScript(data, env) {
  const names = Object.keys(env);
  const bind = names.length ? `const { ${names.join(', ')} } = globalThis.__ytjsEnv;` : '';
  const collect = data.exported.map((k) => `${JSON.stringify(k)}:${k}`).join(',');
  const source = `(function(){\n${bind}\n${data.output}\nreturn {${collect}};\n})()`;
  globalThis.__ytjsEnv = env;
  try {
    return runScript(source);
  } finally {
    delete globalThis.__ytjsEnv;
  }
}

// Must be installed before any Innertube session is created, since the shim is
// read at decipher time from whatever was loaded last.
Platform.load({ ...Platform.shim, eval: evaluateScript });

/**
 * Get BotGuard's interpreter running, whatever the host page's CSP allows.
 *
 * youtube.com sends `require-trusted-types-for 'script'`, so assigning
 * `script.textContent` throws outright. Each route is VERIFIED rather than
 * assumed: a CSP that blocks inline execution lets the element append without
 * throwing and simply never runs it, so "no exception" is not evidence.
 */
function loadInterpreter(source, hash, globalName) {
  const ran = () => !!globalName && !!globalThis[globalName];
  if (ran()) return true;

  const mkScript = () => {
    const s = document.createElement('script');
    s.type = 'text/javascript';
    if (hash) s.id = hash;
    return s;
  };
  const tryRoute = (fill) => {
    try {
      const el = mkScript();
      fill(el);
      document.head.appendChild(el);
      if (ran()) return true;
      el.remove();
    } catch {
      /* next route */
    }
    return false;
  };

  // Shared with the deciphering evaluator: some CSPs cap how many policies a
  // document may create, so one is created and reused.
  const policy = getScriptPolicy();
  if (policy) {
    if (tryRoute((el) => (el.textContent = policy.createScript(source)))) return true;
  }
  // `textContent` is a Trusted Types sink; createTextNode + appendChild is not.
  if (tryRoute((el) => el.appendChild(document.createTextNode(source)))) return true;
  if (tryRoute((el) => (el.textContent = source))) return true;

  // Always runs, but in FUNCTION scope, so a top-level `var` would not become
  // the global the challenge names. Last for that reason.
  try {
    new Function(source)();
  } catch {
    /* fall through */
  }
  return ran();
}

/** Mint a PO token bound to `contentBinding`. */
async function mintToken(contentBinding) {
  const raw = await getChallenge({
    requestKey: WAA_REQUEST_KEY,
    fetchFunction: boundFetch,
    useYouTubeAPI: true,
  });
  const challenge = raw?.bgChallenge ?? raw;
  if (!challenge?.program) throw new Error('no challenge program');

  let interpreter =
    challenge.interpreterJavascript?.privateDoNotAccessOrElseSafeScriptWrappedValue;
  if (!interpreter) {
    const url = challenge.interpreterUrl?.privateDoNotAccessOrElseTrustedResourceUrlWrappedValue;
    if (!url) throw new Error('no interpreter');
    interpreter = await (await fetch(url.startsWith('//') ? `https:${url}` : url)).text();
  }
  if (!loadInterpreter(interpreter, challenge.interpreterHash, challenge.globalName)) {
    throw new Error(`interpreter did not expose '${challenge.globalName}'`);
  }

  const client = await BotGuardClient.create({
    globalObject: globalThis,
    globalName: challenge.globalName,
    program: challenge.program,
  });
  const webPoSignalOutput = [];
  const botguardResponse = await client.snapshot({ webPoSignalOutput });

  const itRes = await fetch(buildURL('GenerateIT', true), {
    method: 'POST',
    headers: {
      'content-type': 'application/json+protobuf',
      'x-goog-api-key': GOOG_API_KEY,
      'x-user-agent': 'grpc-web-javascript/0.1',
    },
    body: JSON.stringify([WAA_REQUEST_KEY, botguardResponse]),
  });
  const integrityToken = (await itRes.json())?.[0];
  if (!integrityToken) throw new Error('no integrity token');

  const minter = await WebPoMinter.create({ integrityToken }, webPoSignalOutput);
  return await minter.mintAsWebsafeString(contentBinding);
}

/** The server answers with a redirect body while `alr=yes` is set. */
function forceNoRedirect(url) {
  return url.includes('alr=yes') ? url.replace('alr=yes', 'alr=no') : `${url}&alr=no`;
}

const shortEdge = (f) => (f.width ? Math.min(f.height || 0, f.width) : f.height || 0);

// The expensive half of a resolve, kept for reuse.
//
// Playback re-issues its urls every ~15 seconds, and ONLY the per-request url
// signing actually expires — the visitor data, the PO token (hours) and the
// Innertube session with its parsed player JS do not. Rebuilding them per
// rotation meant running BotGuard, an obfuscated integrity interpreter, on a
// fixed cadence next to a playing video: measured as a periodic stutter whose
// period tracked the rotation interval.
let warmSession = null;

async function warmFor(videoId, fresh) {
  if (!fresh && warmSession && warmSession.videoId === videoId) return warmSession;
  // A throwaway session first, purely to learn this browser's visitor data so
  // the token and the real session agree on identity.
  const bootstrap = await Innertube.create({ retrieve_player: false, fetch: boundFetch });
  const visitorData = bootstrap.session.context.client.visitorData;

  // Bound to the VIDEO ID, which is what Invidious does
  // (`createMinter` posts `{type: 'content-token-request', videoId}`).
  const poToken = await mintToken(videoId);

  const yt = await Innertube.create({
    po_token: poToken,
    visitor_data: visitorData,
    cache: new UniversalCache(false),
    generate_session_locally: false,
    fetch: boundFetch,
  });
  warmSession = { videoId, visitorData, poToken, yt };
  return warmSession;
}

async function resolve(videoId, minHeight, fresh) {
  // Only getInfo is re-run per rotation; everything above it is reused.
  //
  // `fresh` rebuilds the whole session (visitor data + PO token + player). The
  // reused parts do not expire on the rotation timescale, but they DO eventually
  // go stale, and when they do every url this returns is signed with a dead
  // credential and 403s on first fetch. Without a way to discard it, the warm
  // session was terminal: playback fell back to HLS (capped at 1080p) and stayed
  // there until the video changed or the app restarted, which is exactly what
  // "1440p is offered but never applies" looked like.
  const { yt } = await warmFor(videoId, fresh);

  let info;
  try {
    info = await yt.getInfo(videoId);
  } catch (e) {
    // A session that cannot even answer getInfo is not worth keeping for the
    // next rotation.
    warmSession = null;
    throw e;
  }
  const formats = info.streaming_data?.adaptive_formats || [];
  if (!formats.length) throw new Error('no adaptive formats');

  // Tallest by SHORT edge, so a vertical 1080x1920 stream is not mistaken for a
  // 1920p one. VP9 or AV1: both are fMP4 or WebM we can containerise.
  const videos = formats
    .filter((f) => f.has_video && shortEdge(f) > 0)
    .sort((a, b) => shortEdge(b) - shortEdge(a) || (b.bitrate || 0) - (a.bitrate || 0));
  // AAC in MP4, never the Opus-in-WebM the browser picks: only the MP4 one can
  // be served to hls.js without another transmux.
  const audios = formats
    .filter((f) => f.has_audio && !f.has_video && String(f.mime_type || '').includes('mp4a'))
    .sort((a, b) => (b.bitrate || 0) - (a.bitrate || 0));

  const audio = audios[0];
  if (!videos.length) throw new Error('no video formats');
  if (!audio) throw new Error('no AAC audio track to pair with');

  const player = yt.session.player;
  const audioUrl = forceNoRedirect(await audio.decipher(player));

  // Decipher every rendition ABOVE the cap, not just the tallest, so the quality
  // menu can offer a real choice. Deciphering is cheap once the player is
  // loaded, and there are rarely more than two of these.
  const wanted = videos.filter((f) => shortEdge(f) > minHeight);
  const out = [];
  for (const f of wanted) {
    const h = shortEdge(f);
    const fps = f.fps || 30;
    out.push({
      url: forceNoRedirect(await f.decipher(player)),
      itag: f.itag,
      width: f.width || 0,
      height: f.height || 0,
      fps,
      mimeType: String(f.mime_type || ''),
      name: fps >= 50 ? `${h}p${Math.round(fps)}` : `${h}p`,
      quality: h,
    });
  }
  if (!out.length) {
    throw new Error(`nothing above ${minHeight}p (offered: ${[...new Set(videos.map(shortEdge))].join(', ')})`);
  }

  return {
    videos: out,
    audioUrl,
    audioItag: audio.itag,
  };
}

const report = (frag) => {
  try {
    location.hash = frag;
  } catch {
    /* window may already be closing */
  }
};

// Re-resolve without navigating. The host calls this for every rotation after
// the first; a page load would throw away the warm session above, which is the
// whole point of keeping it.
globalThis.__snResolve = async (videoId, minHeight, fresh) => {
  try {
    const json = JSON.stringify(await resolve(videoId, minHeight, fresh));
    report('#YT_STREAMS=' + btoa(String.fromCharCode(...new TextEncoder().encode(json))));
  } catch (e) {
    report('#YT_ERR=' + encodeURIComponent(String((e && e.message) || e)));
  }
};

(async () => {
  try {
    const videoId = decodeURIComponent((location.hash.match(/VIDEO=([^&]+)/) || [])[1] || '');
    if (!videoId) return report('#YT_ERR=' + encodeURIComponent('no video id'));
    const minHeight = parseInt((location.hash.match(/MIN=(\d+)/) || [])[1] || '1080', 10);
    const json = JSON.stringify(await resolve(videoId, minHeight));
    // base64 so no fragment escaping can corrupt the URLs.
    report('#YT_STREAMS=' + btoa(String.fromCharCode(...new TextEncoder().encode(json))));
  } catch (e) {
    report('#YT_ERR=' + encodeURIComponent(String((e && e.message) || e)));
  }
})();
