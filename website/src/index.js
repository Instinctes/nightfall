/**
 * NIGHTFALLCOIN — website Worker.
 *
 * The site is entirely static, so this Worker does one useful thing: it puts
 * strict security headers on every response.
 *
 * That is not decoration for a project like this. The page serves wallet
 * binaries. If someone could inject a script into it, they could swap a
 * download link and drain everyone who trusted the domain. A strict
 * `script-src 'self'` makes that class of attack fail even if markup is
 * somehow compromised — and it costs nothing, because the site deliberately
 * loads no third-party code at all.
 */

const SECURITY_HEADERS = {
    // Nothing external is loaded, so the policy can be strict. `unsafe-inline`
    // is permitted for styles only — the markup carries a handful of inline
    // style attributes, and injected CSS cannot exfiltrate or execute.
    "Content-Security-Policy": [
        "default-src 'self'",
        "script-src 'self'",
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
    // A page about privacy has no business asking for any of these.
    "Permissions-Policy":
        "accelerometer=(), camera=(), geolocation=(), gyroscope=(), " +
        "magnetometer=(), microphone=(), payment=(), usb=(), interest-cohort=()",
    "Cross-Origin-Opener-Policy": "same-origin",
    "Cross-Origin-Resource-Policy": "same-origin",
};

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
            ...SECURITY_HEADERS,
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
    if (/\.(css|js|svg|png|woff2?)$/i.test(pathname)) {
        return "public, max-age=86400";
    }
    return "public, max-age=300, must-revalidate";
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

        // Only reads. The site has no forms and no API.
        if (request.method !== "GET" && request.method !== "HEAD") {
            return new Response("Method not allowed", {
                status: 405,
                headers: { Allow: "GET, HEAD", ...SECURITY_HEADERS },
            });
        }

        const asset = await env.ASSETS.fetch(request);
        const headers = new Headers(asset.headers);

        for (const [key, value] of Object.entries(SECURITY_HEADERS)) {
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
