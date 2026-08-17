/**
 * NIGHTFALLCOIN — website Worker.
 *
 * Two jobs:
 *   1. Strict security headers on every response. The page serves wallet
 *      binaries. If someone could inject a script into it, they could swap a
 *      download link and drain everyone who trusted the domain.
 *   2. POST /wallet-api — a same-origin proxy onto the seed's light HTTP
 *      API. The browser wallet is served over HTTPS and cannot speak
 *      `http://seed:17888` (mixed content). Cloudflare `fetch()` only
 *      reaches ports 80/443, so the seed forwards :80 → :17888.
 */

const MOBILE_UPSTREAM = "http://seed.nightfallcoin.org/";
const MOBILE_ALLOWED = new Set([
    "status",
    "scan_feed",
    "submit_tx",
    "get_utxo_root",
    "banner",
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
        const upstream = await fetch(MOBILE_UPSTREAM, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body,
        });
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

async function proxySupply() {
    try {
        const upstream = await fetch(MOBILE_UPSTREAM, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ method: "status", params: {}, id: 1 }),
        });
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
            difficulty: r.difficulty,
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

export default {
    async fetch(request, env) {
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
