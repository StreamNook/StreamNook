// Injected into the hidden tiktok.com window that backs TikTok's live directory.
//
// TikTok signs its web API calls inside the page: the site's own fetch wrapper
// appends X-Bogus / X-Gnarly / X-Dynosaur / msToken, and a request built outside
// the page is refused. So the directory is read FROM the page, by cloning the
// feed request the page itself makes and changing only what we want to ask for.
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
  if (window.__snTikTokFeed) return;

  // Signature parameters belong to the page's original request. They are
  // removed so the page signs the modified one afresh.
  const SIGNATURES = ['X-Bogus', 'X-Gnarly', 'X-Dynosaur', 'msToken'];

  // The page only fires its feed request once it has laid out, which is why the
  // hidden window has real dimensions. Its parameters describe this browser in
  // full, and a hand-built subset is refused, so it is the only safe template.
  const template = () => {
    try {
      return (
        performance
          .getEntriesByType('resource')
          .map((e) => e.name)
          .find((n) => n.includes('/webcast/feed/')) || null
      );
    } catch (_) {
      return null;
    }
  };

  const waitForTemplate = async (ms) => {
    const until = Date.now() + ms;
    for (;;) {
      const t = template();
      if (t) return t;
      if (Date.now() > until) return null;
      await new Promise((r) => setTimeout(r, 250));
    }
  };

  const answer = (id, body) => {
    try {
      location.hash = 'SNFEED=' + id + ':' + encodeURIComponent(JSON.stringify(body));
    } catch (_) {}
  };

  const firstUrl = (image) => (image && image.url_list && image.url_list[0]) || null;
  // A frame of the stream itself, portrait like the stream. Carried under `urls`
  // rather than the `url_list` every other image here uses, and absent on some
  // rooms, whose card then falls back to `cover` (which is the avatar).
  const snapshot = (room) =>
    (room.stream_snapshot && room.stream_snapshot.urls && room.stream_snapshot.urls[0]) || null;

  window.__snTikTokFeed = async (id, keywords) => {
    try {
      const tpl = await waitForTemplate(20000);
      if (!tpl) return answer(id, { ok: false, error: 'the page never requested its live feed' });
      const url = new URL(tpl);
      for (const k of SIGNATURES) url.searchParams.delete(k);
      url.searchParams.set('search_keywords', keywords);
      const res = await fetch(url.toString(), { credentials: 'include' });
      if (!res.ok) return answer(id, { ok: false, error: 'the feed answered HTTP ' + res.status });
      const json = await res.json();
      if (json.status_code !== 0) {
        return answer(id, { ok: false, error: 'the feed answered status ' + json.status_code });
      }
      const rows = (json.data || [])
        .map((entry) => entry && entry.data)
        // 2 is live. The feed occasionally carries a room that has just ended.
        .filter((room) => room && room.status === 2 && room.owner && room.owner.display_id)
        .map((room) => ({
          roomId: String(room.id_str || ''),
          handle: String(room.owner.display_id),
          nickname: room.owner.nickname || null,
          userId: room.owner.id_str || null,
          title: room.title || null,
          viewers: typeof room.user_count === 'number' ? room.user_count : null,
          cover: firstUrl(room.cover),
          snapshot: snapshot(room),
          avatar: firstUrl(room.owner.avatar_thumb),
          category: (room.hashtag && room.hashtag.title) || null,
          ageRestricted: !!(
            room.age_restricted &&
            (room.age_restricted === true || room.age_restricted.restricted)
          ),
        }));
      answer(id, { ok: true, rows });
    } catch (e) {
      answer(id, { ok: false, error: String((e && e.message) || e) });
    }
  };
})();
