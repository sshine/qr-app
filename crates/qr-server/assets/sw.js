// The service worker.
//
// Scoped at the root so it covers /c/, which is where every page a visitor sees lives.
//
// What it caches and what it refuses to cache is the whole design. Pages and API
// responses are capability-protected and must never sit in a cache that outlives the
// cookie, so they are network-only. Assets are requested with a `?v=` that changes with
// their content, and photo derivatives are content-addressed, so a cached URL can never
// yield different bytes. That is what makes cache-first correct here.
//
// Bump VERSION whenever this file's caching rules change. It is what evicts a shell
// cached by a previous version, including one that stored assets the new pages no
// longer ask for.

const VERSION = "qr-v2";
const SHELL = `${VERSION}-shell`;
const MEDIA = `${VERSION}-media`;

self.addEventListener("install", (event) => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((names) =>
        Promise.all(
          names
            .filter((n) => !n.startsWith(VERSION))
            .map((n) => caches.delete(n)),
        ),
      )
      .then(() => self.clients.claim()),
  );
});

/** Thumbnails and display copies, which are content-addressed on the server. */
function isMedia(url) {
  return /\/api\/v1\/h\/[a-z0-9-]+\/photos\/[0-9a-f-]+\/(thumb|view)$/.test(
    url.pathname,
  );
}

function isAsset(url) {
  return url.pathname.startsWith("/assets/");
}

self.addEventListener("fetch", (event) => {
  const { request } = event;
  if (request.method !== "GET") return;

  const url = new URL(request.url);
  if (url.origin !== location.origin) return;

  // Cache-first, because the bytes behind these URLs never change.
  if (isAsset(url) || isMedia(url)) {
    event.respondWith(
      caches.match(request).then(
        (hit) =>
          hit ??
          fetch(request).then((response) => {
            // Only a real answer is worth keeping: an opaque or error response cached
            // here would be served for a year.
            if (response.ok && response.status === 200) {
              const copy = response.clone();
              caches
                .open(isAsset(url) ? SHELL : MEDIA)
                .then((c) => c.put(request, copy));
            }
            return response;
          }),
      ),
    );
    return;
  }

  // Everything else — pages, the API — goes to the network and is not stored. A page
  // held here would outlive the capability that fetched it.
});
