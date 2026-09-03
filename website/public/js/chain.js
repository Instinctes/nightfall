/* Chain view.
 *
 * Reads two same-origin endpoints and renders what a privacy chain can
 * honestly show. No dependencies: the site's policy is `script-src 'self'`,
 * which rules out a chart library from a CDN, and a 60-point line does not
 * justify shipping one anyway.
 *
 * Everything goes in through textContent. The data comes from our own node,
 * but a viewer that would render markup from a node is a viewer that breaks
 * the day someone else's node answers.
 */
(function () {
    "use strict";

    var DARKS = 1e8;           // 1 NIGHT
    var TARGET = 15;           // seconds per block
    // Blocks land every 15 s. Polling slower than that guarantees the page is
    // usually showing something that has already been superseded.
    var REFRESH_MS = 10000;
    var WANT = 60;             // headers to ask for

    var lastOk = 0;            // when data last arrived, unix seconds
    var lastTip = 0;           // highest block already on screen
    var inFlight = false;

    function $(id) { return document.getElementById(id); }

    function int(n) {
        if (n === null || n === undefined || isNaN(n)) return "—";
        return Number(n).toLocaleString("en-US");
    }

    /** Darks to NIGHT, grouped, without a wall of trailing zeros. */
    function night(darks, decimals) {
        if (darks === null || darks === undefined || isNaN(darks)) return "—";
        var d = decimals === undefined ? 2 : decimals;
        return (Number(darks) / DARKS).toLocaleString("en-US", {
            minimumFractionDigits: d,
            maximumFractionDigits: d,
        });
    }

    function ago(unix) {
        if (!unix) return "—";
        var s = Math.max(0, Math.floor(Date.now() / 1000) - Number(unix));
        if (s < 60) return s + "s ago";
        if (s < 3600) return Math.floor(s / 60) + "m ago";
        if (s < 86400) return Math.floor(s / 3600) + "h ago";
        return Math.floor(s / 86400) + "d ago";
    }

    function shortHash(h) {
        if (!h || h.length < 16) return h || "—";
        return h.slice(0, 8) + "…" + h.slice(-6);
    }

    function setText(id, value) {
        var el = $(id);
        if (el) el.textContent = value;
    }

    function showError(msg) {
        var slot = $("err-slot");
        if (!slot) return;
        slot.textContent = "";
        if (!msg) return;
        var d = document.createElement("div");
        d.className = "c-err";
        d.textContent = msg;
        slot.appendChild(d);
    }

    /* ------------------------------------------------------- network -- */

    function renderStatus(n) {
        setText("s-tip", int(n.tip_height));
        setText("s-age", ago(n.tip_time));
        // Kept so the per-second ticker below can re-render the age without
        // another round trip.
        if ($("s-age")) $("s-age").dataset.t = String(n.tip_time || "");
        setText("s-diff", int(n.difficulty));
        setText("s-mem", int(n.mempool));
        setText("s-peers", int(n.peers));

        // Green only when the node is caught up and the chain is moving. A
        // tip older than eight target intervals is worth a colour change, not
        // an alarm — a single slow block is normal.
        var dot = $("dot");
        if (dot) {
            var age = Math.floor(Date.now() / 1000) - Number(n.tip_time || 0);
            dot.className = "c-dot " +
                (n.loading ? "warn" : age > TARGET * 20 ? "bad" : age > TARGET * 8 ? "warn" : "ok");
        }

        // Supply proof.
        var badge = $("inv-badge");
        var ok = !!n.supply_invariant_ok && !n.loading;
        if (badge) {
            badge.className = "c-badge " + (ok ? "ok" : "bad");
            badge.firstChild.className = "c-dot " + (ok ? "ok" : "bad");
        }
        setText("inv-text", n.loading
            ? "node still loading"
            : ok ? "verified by this node" : "NOT verified");

        var cap = Number(n.max_supply || 90000000) * DARKS;
        var circ = Number(n.circulating || 0);
        var mint = Number(n.minted || 0);
        var burn = Number(n.burned_fees || 0);

        setText("v-circ", night(circ) + " NIGHT");
        setText("v-mint", night(mint) + " NIGHT");
        setText("v-burn", night(burn, 4) + " NIGHT");
        setText("v-pct", ((mint / cap) * 100).toFixed(4) + "%");

        // Against the cap the issued share is a sliver, which is the honest
        // picture. Burned fees get their own scale or they would be invisible.
        width("f-circ", (circ / cap) * 100);
        width("f-mint", (mint / cap) * 100);
        width("f-burn", mint > 0 ? (burn / mint) * 100 : 0);

        setText("v-utxos", int(n.utxos));
        setText("v-kernels", int(n.kernels));
        setText("v-work", n.total_work ? int(n.total_work) : "—");
        setText("v-root", shortHash(n.utxo_root));

        renderVersions(n.peer_versions || {});
    }

    function width(id, pct) {
        var el = $(id);
        if (!el) return;
        // Anything above zero gets a hairline so "small" never renders as
        // "none" — but the floor stays low on purpose. At 0.46 % issued the
        // first version clamped every bar to the same 0.6 % and three very
        // different numbers came out looking identical. A bar that lies to
        // make itself visible is worse than a bar that is nearly empty:
        // nearly empty *is* the story.
        var p = Math.max(0, Math.min(100, pct));
        el.style.width = (p > 0 && p < 0.12 ? 0.12 : p) + "%";
    }

    function renderVersions(map) {
        var box = $("versions");
        if (!box) return;
        var rows = Object.keys(map).map(function (k) {
            return { name: k, n: Number(map[k]) || 0 };
        }).sort(function (a, b) { return b.n - a.n; });
        var total = rows.reduce(function (s, r) { return s + r.n; }, 0);
        box.textContent = "";
        if (!rows.length) {
            box.appendChild(document.createTextNode("No peers connected right now."));
            return;
        }
        rows.forEach(function (r) {
            var row = document.createElement("div");
            row.className = "c-ver";

            var name = document.createElement("span");
            name.className = "mono";
            name.textContent = r.name;

            var track = document.createElement("span");
            track.className = "c-track";
            var fill = document.createElement("span");
            fill.className = "c-fill";
            fill.style.width = (total ? (r.n / total) * 100 : 0) + "%";
            track.appendChild(fill);

            var num = document.createElement("span");
            num.className = "n";
            num.textContent = String(r.n);

            row.appendChild(name);
            row.appendChild(track);
            row.appendChild(num);
            box.appendChild(row);
        });
    }

    /* -------------------------------------------------------- blocks -- */

    function renderBlocks(headers) {
        var body = $("blocks");
        if (!body) return;
        body.textContent = "";
        if (!headers.length) {
            var tr = document.createElement("tr");
            var td = document.createElement("td");
            td.colSpan = 8;
            td.className = "c-skel";
            td.textContent = "No blocks returned.";
            tr.appendChild(td);
            body.appendChild(tr);
            return;
        }
        headers.slice().reverse().slice(0, 25).forEach(function (h) {
            var tr = document.createElement("tr");
            // Anything above the highest block we had last time is new since
            // the reader last looked, and gets one flash to say so.
            if (lastTip && Number(h.height) > lastTip) tr.className = "c-new";
            cell(tr, int(h.height), "mono");
            var age = cell(tr, ago(h.time), "c-dim");
            // The table is redrawn every ten seconds; the ages have to move
            // every second or the page looks frozen between fetches.
            age.dataset.t = String(h.time || "");
            cell(tr, int(h.difficulty), "mono c-num");
            cell(tr, h.inputs === undefined ? "—" : int(h.inputs), "mono c-num");
            cell(tr, h.outputs === undefined ? "—" : int(h.outputs), "mono c-num");
            cell(tr, h.kernels === undefined ? "—" : int(h.kernels), "mono c-num");
            cell(tr, h.reward === undefined ? "—" : night(h.reward), "mono c-num");
            cell(tr, shortHash(h.hash), "mono c-hash");
            body.appendChild(tr);
        });
    }

    function cell(tr, text, cls) {
        var td = document.createElement("td");
        if (cls) td.className = cls;
        td.textContent = text;
        tr.appendChild(td);
        return td;
    }

    /* --------------------------------------------------------- chart -- */

    function renderChart(headers) {
        var svg = $("chart");
        if (!svg) return;
        svg.textContent = "";
        if (headers.length < 3) return;

        var W = 720, H = 190, PAD = 6;
        var solves = [];
        for (var i = 1; i < headers.length; i++) {
            // Timestamps are miner-stamped and only loosely ordered, so a
            // negative gap is possible and meaningless. Clamp it away.
            solves.push({
                t: Math.max(0, Number(headers[i].time) - Number(headers[i - 1].time)),
                d: Number(headers[i].difficulty),
                h: Number(headers[i].height),
            });
        }
        var maxT = Math.max(TARGET * 2, Math.max.apply(null, solves.map(function (s) { return s.t; })));
        var ds = solves.map(function (s) { return s.d; });
        var minD = Math.min.apply(null, ds), maxD = Math.max.apply(null, ds);
        if (maxD === minD) { maxD = minD + 1; }

        var bw = (W - PAD * 2) / solves.length;

        // Bars: seconds between blocks.
        solves.forEach(function (s, i) {
            var bh = (s.t / maxT) * (H - 30);
            var r = el("rect", {
                x: (PAD + i * bw + bw * 0.18).toFixed(2),
                y: (H - 14 - bh).toFixed(2),
                width: Math.max(1, bw * 0.64).toFixed(2),
                height: Math.max(1, bh).toFixed(2),
                fill: "var(--teal)",
                opacity: s.t > TARGET * 4 ? "0.85" : "0.42",
                rx: "1.5",
            });
            var title = el("title", {});
            title.textContent = "Block " + s.h + " · " + s.t + " s · difficulty " + int(s.d);
            r.appendChild(title);
            svg.appendChild(r);
        });

        // The 15 s target.
        var ty = H - 14 - (TARGET / maxT) * (H - 30);
        svg.appendChild(el("line", {
            x1: PAD, x2: W - PAD, y1: ty.toFixed(2), y2: ty.toFixed(2),
            stroke: "var(--border-hi)", "stroke-width": "1", "stroke-dasharray": "4 4",
        }));

        // Difficulty, scaled to its own range so drift is visible at all.
        var pts = solves.map(function (s, i) {
            var x = PAD + i * bw + bw / 2;
            var y = 14 + (1 - (s.d - minD) / (maxD - minD)) * (H - 58);
            return x.toFixed(2) + "," + y.toFixed(2);
        }).join(" ");
        svg.appendChild(el("polyline", {
            points: pts, fill: "none", stroke: "var(--violet-hi)",
            "stroke-width": "2", "stroke-linejoin": "round", "stroke-linecap": "round",
        }));

        var avg = solves.reduce(function (a, s) { return a + s.t; }, 0) / solves.length;
        setText("chart-avg",
            "Average over these " + solves.length + " blocks: " + avg.toFixed(1) + " s");
        setText("s-avg", avg.toFixed(1) + "s");
    }

    function el(name, attrs) {
        var n = document.createElementNS("http://www.w3.org/2000/svg", name);
        Object.keys(attrs).forEach(function (k) { n.setAttribute(k, attrs[k]); });
        return n;
    }

    /* ---------------------------------------------------------- load -- */

    function getJSON(url) {
        return fetch(url, { cache: "no-store" }).then(function (r) {
            return r.json().then(function (body) {
                if (!r.ok) throw new Error((body && body.error) || ("HTTP " + r.status));
                return body;
            });
        });
    }

    function setLive(state, text) {
        var dot = $("dot");
        if (dot && state) dot.className = "c-dot " + state;
        setText("s-live", text);
    }

    /** Seconds since data last arrived, rendered as a phrase. */
    function freshness() {
        if (!lastOk) return "connecting…";
        var s = Math.floor(Date.now() / 1000) - lastOk;
        if (s <= 1) return "live · just updated";
        if (s < 60) return "live · updated " + s + "s ago";
        return "stale · last update " + Math.floor(s / 60) + "m ago";
    }

    function load() {
        if (inFlight) return;
        inFlight = true;
        var btn = $("refresh");
        if (btn) btn.disabled = true;
        var problems = [];

        var a = getJSON("/network.json").then(renderStatus).catch(function (e) {
            problems.push("Node status unavailable: " + e.message);
        });

        var b = getJSON("/chain.json?limit=" + WANT).then(function (d) {
            var hs = (d.headers || []).slice().sort(function (x, y) {
                return Number(x.height) - Number(y.height);
            });
            renderBlocks(hs);
            renderChart(hs);
            if (hs.length) {
                var top = Number(hs[hs.length - 1].height);
                if (top > lastTip) lastTip = top;
            }
        }).catch(function (e) {
            problems.push("Block headers unavailable: " + e.message);
            var body = $("blocks");
            if (body) {
                body.textContent = "";
                var tr = document.createElement("tr");
                var td = document.createElement("td");
                td.colSpan = 8;
                td.className = "c-skel";
                td.textContent = "Not available from this node right now.";
                tr.appendChild(td);
                body.appendChild(tr);
            }
        });

        Promise.all([a, b]).then(function () {
            inFlight = false;
            if (btn) btn.disabled = false;
            if (problems.length) {
                showError(problems.join(" · "));
                // The pulse stops when the data stops. An animation that keeps
                // running through a failed fetch tells the reader everything
                // is fine while it is not.
                setLive("bad", "not updating — " + problems.length + " endpoint(s) failing");
            } else {
                showError("");
                lastOk = Math.floor(Date.now() / 1000);
                setLive(null, freshness());
            }
        });
    }

    load();
    setInterval(load, REFRESH_MS);

    // Every timestamp on the page moves once a second, without a request.
    // Between two fetches the page would otherwise sit perfectly still and
    // look like a screenshot of a chain rather than a chain.
    setInterval(function () {
        var nodes = document.querySelectorAll("[data-t]");
        for (var i = 0; i < nodes.length; i++) {
            var v = nodes[i].dataset.t;
            if (v) nodes[i].textContent = ago(v);
        }
        if (lastOk) setText("s-live", freshness());
    }, 1000);

    var btn = $("refresh");
    if (btn) btn.addEventListener("click", load);

    // Coming back to a backgrounded tab should not show a minute-old chain.
    // Browsers throttle timers in hidden tabs, so the interval alone is not
    // enough.
    document.addEventListener("visibilitychange", function () {
        if (!document.hidden) load();
    });
    window.addEventListener("focus", load);
    window.addEventListener("online", load);
})();

/* ------------------------------------------------------------ bootstrap ---
 *
 * The chain archive, described from its own manifest rather than from
 * numbers typed into the page.
 *
 * A hardcoded height goes stale the moment the next archive is published,
 * and a stale figure here is worse than none: it tells a newcomer they are
 * downloading 95k blocks when they are downloading 80k, and they only find
 * out after the sync they were trying to avoid. So the page asks the file.
 *
 * If no archive has been published yet, the section says so plainly instead
 * of showing dashes forever.
 */
(function bootstrapPanel() {
    // Deliberately not under /downloads/. That directory is checked against
    // the release version by scripts/check-download-links.mjs, and this file
    // carries a chain height rather than a release number — putting it there
    // would mean weakening a check to fit a file that is metadata, not a
    // download.
    const MANIFEST = "/chain/bootstrap.json";
    const set = (id, text) => {
        const el = document.getElementById(id);
        if (el) el.textContent = text;
    };

    fetch(MANIFEST, { cache: "no-cache" })
        .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
        .then((m) => {
            set("bs-height", Number(m.height).toLocaleString("en-US"));
            set("bs-date", (m.taken_utc || "").slice(0, 10) || "—");
            const mb = Math.round((m.archive_bytes || 0) / 1048576);
            set("bs-size", mb ? `${mb} MB` : "—");
            const sha = m.blocks_bin_sha256 || "";
            set("bs-sha", sha ? sha.slice(0, 16) + "…" : "—");

            const link = document.getElementById("bs-link");
            if (link && m.url) link.href = m.url;

            // The file name goes in its own element rather than into the
            // link's text. Written as one string it made a 513px button on a
            // 375px phone: `.btn` is `white-space: nowrap`, correct for
            // "Download" and wrong for a 40-character archive name, and the
            // overflow gave the whole page a sideways scrollbar. As a second
            // line it wraps and the button stays inside the screen.
            const file = document.getElementById("bs-file");
            if (file && m.archive) {
                file.textContent = m.archive;
                file.hidden = false;
            }
        })
        .catch(() => {
            // No manifest yet. Say that, rather than leaving four dashes and
            // a download button that goes to an empty release.
            for (const id of ["bs-height", "bs-date", "bs-size", "bs-sha"]) set(id, "—");
            const note = document.getElementById("bs-missing");
            if (note) note.hidden = false;
            const link = document.getElementById("bs-link");
            if (link) {
                link.setAttribute("aria-disabled", "true");
                link.style.opacity = "0.5";
                link.style.pointerEvents = "none";
            }
        });
})();
