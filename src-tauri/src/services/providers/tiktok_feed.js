// Injected into the hidden tiktok.com window that signs TikTok's live directory
// requests.
//
// TikTok signs its web API calls inside the page: the site's own fetch wrapper
// appends X-Gnarly / X-Dynosaur / msToken, and a request built outside the page
// is refused. A request the page HAS signed, though, is accepted again when sent
// from outside under the same user agent. So the page is asked only to sign: it
// makes one feed request per category, cloned from its own, and reports the
// signed URLs it sent. The app fetches the directory with those directly. The
// page's own answer comes back too, trimmed to what a card shows, for the case
// where TikTok will not accept the signed request from outside.
//
// Answers go back through the URL fragment, which Rust reads off the window. No
// command is exposed to this origin at all.
(() => {
  // Nothing in this window is ever meant to be seen or heard. The explore page
  // renders static cards today, but a hidden window that started playing a live
  // preview would decode video and play audio nobody can find to stop. Refused
  // in every frame, before any page script runs.
  try {
    HTMLMediaElement.prototype.play = function () {
      this.muted = true;
      return Promise.reject(new DOMException('media is disabled here', 'NotAllowedError'));
    };
  } catch (_) {}

  if (window.top !== window) return;
  if (window.__snTikTokSign) return;

  // The signed URL is read back from resource timing, which by default stops
  // recording after 250 entries. This page loads more than that on its own.
  try {
    performance.setResourceTimingBufferSize(5000);
  } catch (_) {}

  // Signature parameters belong to the page's original request. They are
  // removed so the page signs the modified one afresh.
  const SIGNATURES = ['X-Bogus', 'X-Gnarly', 'X-Dynosaur', 'msToken'];

  const feedEntries = () => {
    try {
      return performance.getEntriesByType('resource').filter((e) => e.name.includes('/webcast/feed/'));
    } catch (_) {
      return [];
    }
  };

  // The page only fires its feed request once it has laid out, which is why the
  // hidden window has real dimensions. Its parameters describe this browser in
  // full, and a hand-built subset is refused, so it is the only safe template.
  const waitForTemplate = async (ms) => {
    const until = Date.now() + ms;
    for (;;) {
      const first = feedEntries()[0];
      if (first) return first.name;
      if (Date.now() > until) return null;
      await new Promise((r) => setTimeout(r, 250));
    }
  };

  const isSigned = (url) => SIGNATURES.some((k) => url.searchParams.has(k));

  // The URL the page actually sent for `keyword`, signatures and all. The entry
  // is recorded as the response finishes, so it can trail the fetch slightly.
  const sentFor = async (keyword, since) => {
    for (let i = 0; i < 20; i++) {
      const hit = feedEntries()
        .filter((e) => e.startTime >= since)
        .map((e) => new URL(e.name))
        .filter((u) => u.searchParams.get('search_keywords') === keyword && isSigned(u))
        .pop();
      if (hit) return hit.toString();
      await new Promise((r) => setTimeout(r, 100));
    }
    return null;
  };

  const text = (v) => (typeof v === 'string' && v ? v : null);
  const firstUrl = (image) => text(image && image.url_list && image.url_list[0]);
  // A frame of the stream itself, carried under `urls` rather than the
  // `url_list` every other image here uses.
  const snapshot = (room) => text(room.stream_snapshot && room.stream_snapshot.urls && room.stream_snapshot.urls[0]);

  const rowsOf = (json) =>
    (json.data || [])
      .map((entry) => entry && entry.data)
      // 2 is live. The feed occasionally carries a room that has just ended.
      .filter((room) => room && room.status === 2 && room.owner && room.owner.display_id)
      .map((room) => ({
        roomId: String(room.id_str || ''),
        handle: String(room.owner.display_id),
        nickname: text(room.owner.nickname),
        userId: text(room.owner.id_str),
        title: text(room.title),
        viewers: typeof room.user_count === 'number' ? room.user_count : null,
        cover: firstUrl(room.cover),
        snapshot: snapshot(room),
        avatar: firstUrl(room.owner.avatar_thumb),
        category: text(room.hashtag && room.hashtag.title),
      }));

  const answer = (id, body) => {
    try {
      location.hash = 'SNFEED=' + id + ':' + encodeURIComponent(JSON.stringify(body));
    } catch (_) {}
  };

  // Sign one category: request it through the page, confirm TikTok accepted
  // it, and return the URL that went out (when it can be read back) with the
  // answer it got.
  const signOne = async (tpl, keyword) => {
    const url = new URL(tpl);
    for (const k of SIGNATURES) url.searchParams.delete(k);
    url.searchParams.set('search_keywords', keyword);
    const since = performance.now();
    const res = await fetch(url.toString(), { credentials: 'include' });
    if (!res.ok) throw new Error('the feed answered HTTP ' + res.status);
    const body = await res.text();
    // TikTok's quiet refusal is an empty 200.
    if (!body) throw new Error('the feed answered with nothing');
    const json = JSON.parse(body);
    if (json.status_code !== 0) throw new Error('the feed answered status ' + json.status_code);
    return { url: await sentFor(keyword, since), rows: rowsOf(json) };
  };

  // The page's OWN request for one feed (by `channel_id`), exactly as it signed
  // and sent it. For a feed the signed-in session decides, like Following, the
  // request is taken as the page made it rather than cloned from another.
  window.__snTikTokCapture = async (id, channelId) => {
    try {
      const until = Date.now() + 20000;
      for (;;) {
        const hit = feedEntries()
          .map((e) => new URL(e.name))
          .filter((u) => u.searchParams.get('channel_id') === String(channelId) && isSigned(u))
          .pop();
        if (hit) return answer(id, { ok: true, ua: navigator.userAgent, signed: { feed: hit.toString() }, rows: {} });
        if (Date.now() > until) return answer(id, { ok: false, error: 'the page never requested that feed' });
        await new Promise((r) => setTimeout(r, 250));
      }
    } catch (e) {
      answer(id, { ok: false, error: String((e && e.message) || e) });
    }
  };

  // Parameters that belong to a feed request and not to a search.
  const FEED_ONLY = ['channel_id', 'req_from', 'search_keywords', 'max_time', 'related_live_tag', 'content_type', 'need_room_count'];

  // One LIVE search, the request TikTok's own search page makes for its LIVE
  // tab. Built from the page's feed request, which carries every parameter the
  // API expects, and signed by the page's fetch wrapper on the way out. Each
  // query is a new request, so TikTok's answer goes back as it came, for Rust
  // to read.
  window.__snTikTokSearch = async (id, query, count) => {
    try {
      const tpl = await waitForTemplate(20000);
      if (!tpl) return answer(id, { ok: false, error: 'the page never requested its live feed' });
      const url = new URL(tpl);
      for (const k of SIGNATURES.concat(FEED_ONLY)) url.searchParams.delete(k);
      url.host = 'www.tiktok.com';
      url.pathname = '/api/search/live/full/';
      url.searchParams.set('keyword', String(query));
      url.searchParams.set('offset', '0');
      url.searchParams.set('count', String(count));
      url.searchParams.set('from_page', 'search');
      const res = await fetch(url.toString(), { credentials: 'include' });
      if (!res.ok) return answer(id, { ok: false, error: 'the search answered HTTP ' + res.status });
      answer(id, { ok: true, body: await res.text() });
    } catch (e) {
      answer(id, { ok: false, error: String((e && e.message) || e) });
    }
  };

  // The request TikTok's own client makes for webcast `path`, with this page's
  // device parameters from its feed request. The page's fetch wrapper signs it
  // and, for a protected POST, adds its CSRF token.
  const webcastRequest = (tpl, path) => {
    const url = new URL(tpl);
    for (const k of SIGNATURES.concat(FEED_ONLY)) url.searchParams.delete(k);
    url.pathname = path;
    return url;
  };

  // Rooms this page has entered, so each is entered once.
  const entered = new Set();

  // One chat message as the signed-in account, the way TikTok's own room page
  // sends one: enter the room (once), then post the message as typed text
  // (input_type 0). TikTok's answers go back as they came, for Rust to read.
  window.__snTikTokChat = async (id, roomId, content) => {
    try {
      const tpl = await waitForTemplate(20000);
      if (!tpl) return answer(id, { ok: false, error: 'the page never requested its live feed' });
      const room = String(roomId);
      let enter = null;
      if (!entered.has(room)) {
        const url = webcastRequest(tpl, '/webcast/room/enter/');
        url.searchParams.set('device_type', 'web_h264');
        const res = await fetch(url.toString(), {
          method: 'POST',
          credentials: 'include',
          headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
          body: new URLSearchParams({ room_id: room, enter_source: 'others-others' }).toString(),
        });
        enter = await res.text().catch(() => null);
        try {
          if (JSON.parse(enter).status_code === 0) entered.add(room);
        } catch (_) {}
      }
      const message = {
        room_id: room,
        content: String(content),
        input_type: 0,
        client_start_timestamp_millisecond: Date.now(),
      };
      const url = webcastRequest(tpl, '/webcast/room/chat/');
      for (const [k, v] of Object.entries(message)) url.searchParams.set(k, String(v));
      const res = await fetch(url.toString(), {
        method: 'POST',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(message),
      });
      if (!res.ok) return answer(id, { ok: false, error: 'the chat answered HTTP ' + res.status, enter });
      answer(id, { ok: true, body: await res.text(), enter });
    } catch (e) {
      answer(id, { ok: false, error: String((e && e.message) || e) });
    }
  };

  window.__snTikTokSign = async (id, keywords) => {
    try {
      const tpl = await waitForTemplate(20000);
      if (!tpl) return answer(id, { ok: false, error: 'the page never requested its live feed' });
      const results = await Promise.allSettled(keywords.map((kw) => signOne(tpl, kw)));
      const signed = {};
      const rows = {};
      let failure = null;
      results.forEach((r, i) => {
        if (r.status === 'fulfilled') {
          if (r.value.url) signed[keywords[i]] = r.value.url;
          rows[keywords[i]] = r.value.rows;
        } else failure = failure || String((r.reason && r.reason.message) || r.reason);
      });
      if (!Object.keys(rows).length) return answer(id, { ok: false, error: failure || 'nothing was fetched' });
      answer(id, { ok: true, ua: navigator.userAgent, signed, rows });
    } catch (e) {
      answer(id, { ok: false, error: String((e && e.message) || e) });
    }
  };
})();
