/**
 * NIGHTFALLCOIN — website Worker.
 *
 * Two jobs:
 *   1. Strict security headers on every response. The page serves wallet
 *      binaries. If someone could inject a script into it, they could swap a
 *      download link and drain everyone who trusted the domain.
 *   2. POST /wallet-api — a same-origin proxy onto the *light* node's HTTP
 *      API (Vultr / seed1). P2P doorbell is seed.nightfallcoin.org (Contabo).
 *      The browser wallet is served over HTTPS and cannot speak
 *      `http://seed:17888` (mixed content). Cloudflare `fetch()` only
 *      reaches ports 80/443, so the light node forwards :80 → :17888.
 */

/** Light nodes the browser and phone wallets read through, in order.
 *
 * One address was one machine, and one machine is one VPS ticket away from
 * every light wallet showing a spinner — which is exactly what happened on
 * 20 August, at two dozen users. The list is tried in order and the first
 * node that answers wins; a dead entry costs one timeout, not an outage.
 *
 * These are *display* only. A light wallet trusts them for what it shows and
 * for nothing else: the seed never leaves the device, and a hostile node can
 * hide a payment or invent one on screen but cannot spend. Adding a name here
 * widens who can lie to a screen, so the list stays short and ours.
 */
const MOBILE_UPSTREAMS = [
    // Two machines, two providers. seed1/seed2 are the same Vultr box;
    // listing both would only add a timeout when that box is down.
    //
    // Contabo first since 23 Aug 2026. It answers `status` in 0.3 s; the
    // Vultr box was taking 8.2 s under an account-wide CPU cap, which is
    // past the timeout below — so every light request was paying six
    // seconds to watch the first entry fail before the second one served
    // it. Order the list by who actually answers, not by who is nominally
    // the light node.
    "http://seed.nightfallcoin.org/",
    "http://seed1.nightfallcoin.org/",
];
const UPSTREAM_TIMEOUT_MS = 6000;

/** POST a JSON-RPC body to the first light node that answers.
 *
 * Returns the upstream `Response`, or throws if every node failed. An
 * upstream that answers with 5xx counts as failed — a proxy that faithfully
 * relays "502" from a node that is still doing initial block download is
 * technically correct and useless to a wallet.
 */
async function lightFetch(body) {
    let lastErr = "no upstream configured";
    for (const url of MOBILE_UPSTREAMS) {
        try {
            const r = await fetch(url, {
                method: "POST",
                headers: { "content-type": "application/json" },
                body,
                signal: AbortSignal.timeout(UPSTREAM_TIMEOUT_MS),
            });
            if (r.ok) {
                return r;
            }
            lastErr = `HTTP ${r.status}`;
        } catch (e) {
            lastErr = (e && e.message) || String(e);
        }
    }
    throw new Error(lastErr);
}
const MOBILE_ALLOWED = new Set([
    "status",
    "scan_feed",
    "submit_tx",
    "get_utxo_root",
    "banner",
    "peers",
]);
const MOBILE_MAX_BODY = 512 * 1024;

function securityHeaders(pathname) {
    // wasm-bindgen instantiates the wallet module with WebAssembly.instantiate,
    // which Chromium treats as eval unless the policy names it.
    const wallet = pathname.startsWith("/wallet");
    return {
        "Content-Security-Policy": [
            "default-src 'self'",
            wallet ? "script-src 'self' 'wasm-unsafe-eval'" : "script-src 'self'",
            "style-src 'self' 'unsafe-inline'",
            "img-src 'self' data:",
            "font-src 'self'",
            "connect-src 'self'",
            "frame-ancestors 'none'",
            "base-uri 'none'",
            "form-action 'none'",
            "object-src 'none'",
            "upgrade-insecure-requests",
        ].join("; "),

        "Strict-Transport-Security": "max-age=31536000; includeSubDomains",
        "X-Content-Type-Options": "nosniff",
        "X-Frame-Options": "DENY",
        "Referrer-Policy": "no-referrer",
        "Permissions-Policy":
            "accelerometer=(), camera=(), geolocation=(), gyroscope=(), " +
            "magnetometer=(), microphone=(), payment=(), usb=(), interest-cohort=()",
        "Cross-Origin-Opener-Policy": "same-origin",
        "Cross-Origin-Resource-Policy": "same-origin",
    };
}

/**
 * One address, not three.
 *
 * The site answers on the apex, on `www` and on the original workers.dev
 * hostname. Serving identical content from several origins is a real problem
 * here rather than a cosmetic one: a user who checks a download link against
 * what a friend sent them should see the same string, and a phishing domain is
 * easier to spot when there is exactly one legitimate spelling. Everything
 * therefore redirects to the apex permanently.
 */
const CANONICAL_HOST = "nightfallcoin.org";

function canonicalRedirect(url) {
    if (url.hostname === CANONICAL_HOST) {
        return null;
    }
    const target = new URL(url);
    target.hostname = CANONICAL_HOST;
    target.protocol = "https:";
    target.port = "";
    return new Response(null, {
        status: 301,
        headers: {
            Location: target.toString(),
            "Cache-Control": "public, max-age=3600",
            ...securityHeaders(url.pathname),
        },
    });
}

function cacheControlFor(pathname) {
    // Download filenames carry the version, so a given URL refers to exactly
    // one file forever and can be cached hard. That is the point of versioning
    // them: previously a release overwrote `…-arm64.dmg` in place, and every
    // cache in between could serve the old bytes under the new build's
    // published checksum — leaving a user who checks the hash unable to tell a
    // stale cache from a tampered download.
    //
    // The checksum list is the exception. It is small, and it is the one thing
    // that must never be stale relative to the files it describes.
    if (pathname.startsWith("/downloads/")) {
        if (pathname.includes("SHA256SUMS")) {
            return "public, max-age=300, must-revalidate";
        }
        return "public, max-age=31536000, immutable";
    }
    if (pathname.startsWith("/wallet/")) {
        // The PWA used to keep a stale wasm that could not send.
        return "no-store";
    }
    if (/\.(css|js|svg|png|woff2?|wasm)$/i.test(pathname)) {
        return "public, max-age=86400";
    }
    return "public, max-age=300, must-revalidate";
}

function jsonResponse(status, obj, extra = {}) {
    return new Response(JSON.stringify(obj), {
        status,
        headers: {
            "Content-Type": "application/json; charset=utf-8",
            "Cache-Control": "no-store",
            ...securityHeaders("/wallet-api"),
            ...extra,
        },
    });
}

async function proxyWalletApi(request) {
    if (request.method === "OPTIONS") {
        return new Response(null, {
            status: 204,
            headers: {
                Allow: "POST, OPTIONS",
                "Cache-Control": "no-store",
                ...securityHeaders("/wallet-api"),
            },
        });
    }
    if (request.method !== "POST") {
        return jsonResponse(
            405,
            { error: "POST a JSON-RPC body" },
            { Allow: "POST, OPTIONS" },
        );
    }

    const text = await request.text();
    if (text.length > MOBILE_MAX_BODY) {
        return jsonResponse(413, { error: "body too large" });
    }
    let payload;
    try {
        payload = JSON.parse(text);
    } catch {
        return jsonResponse(400, { error: "bad json" });
    }
    if (!MOBILE_ALLOWED.has(payload.method)) {
        return jsonResponse(403, {
            error: `method '${payload.method || ""}' is not available on the mobile API`,
        });
    }

    const body = JSON.stringify({
        method: payload.method,
        params: payload.params ?? {},
        id: payload.id ?? 1,
    });

    try {
        const upstream = await lightFetch(body);
        const out = await upstream.text();
        return new Response(out, {
            status: upstream.status,
            headers: {
                "Content-Type": "application/json; charset=utf-8",
                "Cache-Control": "no-store",
                ...securityHeaders("/wallet-api"),
            },
        });
    } catch (e) {
        return jsonResponse(502, {
            error: `node unreachable: ${e.message || e}`,
        });
    }
}

async function proxyPeers() {
    try {
        const upstream = await lightFetch(
            JSON.stringify({ method: "peers", params: {}, id: 1 }),
        );
        const payload = await upstream.json();
        const r = payload && payload.result;
        if (!r || !Array.isArray(r.peers)) {
            return jsonResponse(502, { error: "seed returned no peer list" });
        }
        const body = JSON.stringify({
            peers: r.peers,
            genesis: r.genesis || "",
        });
        return new Response(body, {
            status: 200,
            headers: {
                "Content-Type": "application/json; charset=utf-8",
                "Cache-Control": "public, max-age=30, must-revalidate",
                ...securityHeaders("/peers"),
            },
        });
    } catch (e) {
        return jsonResponse(502, {
            error: `node unreachable: ${e.message || e}`,
        });
    }
}

async function proxyNetwork() {
    try {
        const [statusRes, peersRes] = await Promise.all([
            lightFetch(JSON.stringify({ method: "status", params: {}, id: 1 })),
            lightFetch(JSON.stringify({ method: "peers", params: {}, id: 1 })),
        ]);
        const statusPayload = await statusRes.json();
        const r = statusPayload && statusPayload.result;
        if (!r) {
            return jsonResponse(502, { error: "seed returned no status" });
        }
        let peerCount = 0;
        try {
            const peersPayload = await peersRes.json();
            const pr = peersPayload && peersPayload.result;
            if (pr && Array.isArray(pr.peers)) {
                peerCount = pr.peers.length;
            }
        } catch {
            peerCount = Number(r.live_peers || r.peers || 0);
        }
        const body = JSON.stringify({
            blocks: r.blocks,
            tip_height: r.tip_height,
            tip: r.tip || "",
            difficulty: r.difficulty,
            circulating: r.circulating,
            minted: r.minted,
            burned_fees: r.burned_fees,
            max_supply: r.max_supply,
            supply_invariant_ok: r.supply_invariant_ok,
            peers: peerCount,
            loading: !!r.loading,
            genesis: r.genesis || "",
            protocol_version: r.protocol_version,
            // Everything below is for the chain view. All of it is public
            // bookkeeping — set sizes, roots and the version census — and none
            // of it says anything about who holds what.
            wire_version: r.wire_version,
            utxos: r.utxos,
            kernels: r.kernels,
            utxo_root: r.utxo_root || "",
            total_work: r.total_work || "",
            mempool: r.mempool,
            tip_time: r.tip_time,
            peer_versions: r.peer_versions || {},
            pruned: !!r.pruned,
            ticker: r.ticker || "NIGHT",
            network: r.network || "mainnet",
        });
        return new Response(body, {
            status: 200,
            headers: {
                "Content-Type": "application/json; charset=utf-8",
                "Cache-Control": "public, max-age=15, must-revalidate",
                ...securityHeaders("/network.json"),
            },
        });
    } catch (e) {
        return jsonResponse(502, {
            error: `node unreachable: ${e.message || e}`,
        });
    }
}

async function proxySupply() {
    try {
        const upstream = await lightFetch(
            JSON.stringify({ method: "status", params: {}, id: 1 }),
        );
        const payload = await upstream.json();
        const r = payload && payload.result;
        if (!r) {
            return jsonResponse(502, { error: "seed returned no status" });
        }
        const body = JSON.stringify({
            circulating: r.circulating,
            minted: r.minted,
            burned_fees: r.burned_fees,
            max_supply: r.max_supply,
            supply_invariant_ok: r.supply_invariant_ok,
            tip_height: r.tip_height,
            blocks: r.blocks,
            difficulty: r.difficulty,
            loading: !!r.loading,
        });
        return new Response(body, {
            status: 200,
            headers: {
                "Content-Type": "application/json; charset=utf-8",
                "Cache-Control": "public, max-age=15, must-revalidate",
                ...securityHeaders("/supply"),
            },
        });
    } catch (e) {
        return jsonResponse(502, {
            error: `node unreachable: ${e.message || e}`,
        });
    }
}

/** Block headers for the chain view.
 *
 * Everything this returns is already public in the block. What is *not* here
 * is anything the protocol hides: no amounts, no addresses, no sender and no
 * receiver. There is no way to add them later either — they do not exist on
 * the chain to be read. A viewer that could show them would be a bug report.
 *
 * The upstream caps `limit` at 512; this caps it lower, because the page asks
 * for 60 and nothing about a public endpoint should invite more.
 */
async function proxyChain(url) {
    const limit = Math.min(
        Math.max(parseInt(url.searchParams.get("limit") || "60", 10) || 60, 1),
        200,
    );
    const fromRaw = url.searchParams.get("from");
    const params = { limit };
    if (fromRaw !== null && /^\d+$/.test(fromRaw)) {
        params.from = parseInt(fromRaw, 10);
    }
    try {
        const upstream = await lightFetch(
            JSON.stringify({ method: "get_headers", params, id: 1 }),
        );
        const payload = await upstream.json();
        const r = payload && payload.result;
        if (!r || !Array.isArray(r.headers)) {
            // A seed still on 0.8.2 has no `get_headers` and answers 403 with
            // an error body. Say so plainly instead of rendering an empty
            // table that looks like an empty chain.
            const why =
                (payload && payload.error && (payload.error.message || payload.error)) ||
                "seed returned no headers";
            return jsonResponse(502, { error: String(why) });
        }
        return new Response(JSON.stringify(r), {
            status: 200,
            headers: {
                "Content-Type": "application/json; charset=utf-8",
                "Cache-Control": "public, max-age=15, must-revalidate",
                ...securityHeaders("/chain.json"),
            },
        });
    } catch (e) {
        // `lightFetch` throws on any non-2xx, so a seed that is simply too old
        // to know `get_headers` arrives here as a bare "HTTP 403". Saying
        // "unreachable" about a node that answered immediately and correctly
        // would send whoever reads this looking in the wrong place.
        const msg = String((e && e.message) || e);
        return jsonResponse(502, {
            error:
                msg === "HTTP 403"
                    ? "the seed is older than 0.8.3 and has no get_headers"
                    : `node unreachable: ${msg}`,
        });
    }
}

/** Discord member count, shaped for a shields.io endpoint badge.
 *
 * The obvious way to put a live count on the README is
 * `img.shields.io/discord/<server id>`, and it is the wrong way: it reads
 * Discord's *widget*, and turning the widget on publishes `widget.json`,
 * which enumerates the usernames of everyone currently online. Publishing a
 * list of who is in the room, on a privacy coin, to decorate a badge.
 *
 * The invite endpoint gives the same number to anyone who asks, without a
 * token and without exposing a single member. So the badge is fed from here
 * instead, cached for an hour because nobody needs a live-to-the-second
 * count of a chat room.
 */
const DISCORD_INVITE = "Wj6pTNmVEr";
const DISCORD_BADGE_KEY = "https://nightfallcoin.org/__discord-badge";
const DISCORD_REFRESH_SECS = 1800;

function badgeBody(message) {
    return JSON.stringify({
        schemaVersion: 1,
        label: "discord",
        message,
        color: "5865F2",
    });
}

function badgeResponse(message, fetchedAt) {
    return new Response(badgeBody(message), {
        status: 200,
        headers: {
            "Content-Type": "application/json; charset=utf-8",
            // Long enough that shields.io does not hammer this, short enough
            // that a new member shows up the same day.
            "Cache-Control": "public, max-age=1800",
            "X-Fetched-At": String(fetchedAt),
            // shields.io fetches this cross-origin.
            "Access-Control-Allow-Origin": "*",
            ...securityHeaders("/discord.json"),
        },
    });
}

async function discordBadge(ctx) {
    // Discord refuses roughly a third of the requests that leave a Cloudflare
    // address — shared egress, and the invite endpoint is rate limited per IP.
    // Measured, not assumed: ten calls in a row returned the count seven times
    // and failed three. A badge that flips between "21 members" and "join"
    // depending on which Worker answered is worse than no badge, and shields.io
    // will happily cache whichever it happened to catch.
    //
    // So the last good answer is kept and served whenever upstream declines.
    const cache = caches.default;
    const key = new Request(DISCORD_BADGE_KEY);
    const cached = await cache.match(key);

    if (cached) {
        const age = Date.now() / 1000 - Number(cached.headers.get("X-Fetched-At") || 0);
        if (age < DISCORD_REFRESH_SECS) {
            return cached;
        }
    }

    try {
        const r = await fetch(
            `https://discord.com/api/v10/invites/${DISCORD_INVITE}?with_counts=true`,
            { headers: { "User-Agent": "nightfallcoin.org" } },
        );
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        const d = await r.json();
        const n = d.approximate_member_count;
        if (typeof n !== "number" || n < 0) throw new Error("no count");

        const fresh = badgeResponse(
            `${n} member${n === 1 ? "" : "s"}`,
            Math.floor(Date.now() / 1000),
        );
        // Keep a copy for far longer than we serve it, so a stale number is
        // always available when Discord says no.
        const keep = new Response(fresh.clone().body, fresh);
        keep.headers.set("Cache-Control", "public, max-age=86400");
        if (ctx && ctx.waitUntil) {
            ctx.waitUntil(cache.put(key, keep));
        } else {
            await cache.put(key, keep);
        }
        return fresh;
    } catch {
        // Stale beats wrong. "join" is the last resort and is still true.
        return cached || badgeResponse("join", 0);
    }
}

export default {
    async fetch(request, env, ctx) {
        const url = new URL(request.url);

        // Localhost is exempt so `wrangler dev` does not bounce to production.
        if (url.hostname !== "localhost" && url.hostname !== "127.0.0.1") {
            const redirect = canonicalRedirect(url);
            if (redirect) {
                return redirect;
            }
        }

        // Browser wallet → seed light API. Must run before the GET-only gate.
        if (url.pathname === "/wallet-api" || url.pathname === "/wallet-api/") {
            return proxyWalletApi(request);
        }

        // Homepage supply card. Same seed, a short cache so every visitor
        // does not open a new upstream socket. Numbers are public chain
        // facts — minted, burned, circulating — not a price.
        if (url.pathname === "/supply" || url.pathname === "/supply/") {
            if (request.method !== "GET" && request.method !== "HEAD") {
                return jsonResponse(405, { error: "GET" }, { Allow: "GET, HEAD" });
            }
            return proxySupply();
        }

        // Feeds the README's Discord badge. See discordBadge().
        if (url.pathname === "/discord.json") {
            if (request.method !== "GET" && request.method !== "HEAD") {
                return jsonResponse(405, { error: "GET" }, { Allow: "GET, HEAD" });
            }
            return discordBadge(ctx);
        }

        // Listening nodes a fresh wallet can dial when the compiled-in
        // seed is full. Public facts: addresses that completed a handshake
        // and accepted an outbound from the seed. Not a census of miners.
        if (url.pathname === "/peers" || url.pathname === "/peers/") {
            if (request.method !== "GET" && request.method !== "HEAD") {
                return jsonResponse(405, { error: "GET" }, { Allow: "GET, HEAD" });
            }
            return proxyPeers();
        }

        // The /network/ page was removed on 22 August 2026. Its URL is in the
        // README, in Discord and in posts that are already published, so it
        // redirects rather than 404s — an old link should land somewhere, not
        // nowhere. Exact matches only: /network.json below is a live endpoint
        // and must not be swallowed by a prefix test.
        if (url.pathname === "/network" || url.pathname === "/network/") {
            const home = new URL(url);
            home.pathname = "/";
            home.search = "";
            return new Response(null, {
                status: 301,
                headers: {
                    Location: home.toString(),
                    "Cache-Control": "public, max-age=3600",
                    ...securityHeaders("/"),
                },
            });
        }

        // Public numbers only. No addresses, no graph.
        if (url.pathname === "/network.json") {
            if (request.method !== "GET" && request.method !== "HEAD") {
                return jsonResponse(405, { error: "GET" }, { Allow: "GET, HEAD" });
            }
            return proxyNetwork();
        }

        // Block headers for /chain/. Headers and counts, nothing hidden.
        if (url.pathname === "/chain.json") {
            if (request.method !== "GET" && request.method !== "HEAD") {
                return jsonResponse(405, { error: "GET" }, { Allow: "GET, HEAD" });
            }
            return proxyChain(url);
        }

        // Everything else is static. No forms, no other API.
        if (request.method !== "GET" && request.method !== "HEAD") {
            return new Response("Method not allowed", {
                status: 405,
                headers: {
                    Allow: "GET, HEAD",
                    ...securityHeaders(url.pathname),
                },
            });
        }

        const asset = await env.ASSETS.fetch(request);
        const headers = new Headers(asset.headers);

        for (const [key, value] of Object.entries(securityHeaders(url.pathname))) {
            headers.set(key, value);
        }

        // Cache rules apply to answers, never to failures.
        //
        // `cacheControlFor` hands `/downloads/` a one-year `immutable` lifetime,
        // which is right for a file that exists and catastrophic for one that
        // does not: a single request for a build that has not finished uploading
        // pins that 404 at the edge for a year, and no later deploy dislodges it.
        // The download page was serving exactly that — every link a hard-cached
        // miss. Errors are therefore never stored.
        const ok = asset.status >= 200 && asset.status < 400;
        headers.set(
            "Cache-Control",
            ok ? cacheControlFor(url.pathname) : "no-store",
        );

        // Downloads should save rather than try to render — but only when there
        // is something to save. Attaching a filename to an error page makes a
        // browser write the 404 body to disk under the name of the wallet.
        if (ok && url.pathname.endsWith(".wasm")) {
            headers.set("Content-Type", "application/wasm");
        }

        if (ok && url.pathname.startsWith("/downloads/")) {
            const name = url.pathname.split("/").pop();
            if (name) {
                headers.set("Content-Disposition", `attachment; filename="${name}"`);
            }
        }

        return new Response(asset.body, {
            status: asset.status,
            statusText: asset.statusText,
            headers,
        });
    },
};
