const CACHE = "night-wallet-v1";
const SHELL = ["./", "./index.html", "./style.css", "./app.js", "./pkg/nightfall_web.js", "./pkg/nightfall_web_bg.wasm"];

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});
self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys().then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))).then(() => self.clients.claim()),
  );
});
self.addEventListener("fetch", (e) => {
  const u = new URL(e.request.url);
  if (u.pathname.startsWith("/wallet-api")) return;
  if (e.request.method !== "GET") return;
  e.respondWith(
    caches.match(e.request).then((hit) => hit || fetch(e.request)),
  );
});
