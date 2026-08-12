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

/** Binaries are immutable per release; markup should revalidate. */
function cacheControlFor(pathname) {
    if (pathname.startsWith("/downloads/")) {
        return "public, max-age=3600";
    }
    if (/\.(css|js|svg|png|woff2?)$/i.test(pathname)) {
        return "public, max-age=86400";
    }
    return "public, max-age=300, must-revalidate";
}

export default {
    async fetch(request, env) {
        const url = new URL(request.url);

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
        headers.set("Cache-Control", cacheControlFor(url.pathname));

        // Downloads should save rather than try to render.
        if (url.pathname.startsWith("/downloads/")) {
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
