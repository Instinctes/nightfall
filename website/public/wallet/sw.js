const CACHE = "night-wallet-v16";
const SHELL = ["./index.html", "./style.css"];

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});
self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys().then((keys) => Promise.all(keys.map((k) => caches.delete(k)))).then(() => self.clients.claim()),
  );
});
self.addEventListener("fetch", (e) => {
  const u = new URL(e.request.url);
  if (u.pathname.startsWith("/wallet-api")) return;
  if (e.request.method !== "GET") return;
  // Never cache the wasm or app — a stale module cannot send.
  if (/\.(js|wasm)$/i.test(u.pathname) || u.pathname.endsWith("/app.js")) {
    e.respondWith(fetch(e.request, { cache: "no-store" }));
    return;
  }
  e.respondWith(
    fetch(e.request)
      .then((r) => {
        const copy = r.clone();
        caches.open(CACHE).then((c) => c.put(e.request, copy));
        return r;
      })
      .catch(() => caches.match(e.request)),
  );
});
