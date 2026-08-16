/* NIGHTFALLCOIN — website behaviour.
   No frameworks, no build step, no third-party requests. A page about privacy
   should not phone anyone home. */

(() => {
    "use strict";

    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    /* ------------------------------------------------------ nav on scroll -- */
    const nav = document.getElementById("nav");
    const onScroll = () => nav.classList.toggle("scrolled", window.scrollY > 20);
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });

    /* ---------------------------------------------------- reveal on enter -- */
    const revealables = document.querySelectorAll(".reveal");
    if ("IntersectionObserver" in window && !reduced) {
        const io = new IntersectionObserver(
            (entries) => {
                for (const e of entries) {
                    if (e.isIntersecting) {
                        e.target.classList.add("in");
                        io.unobserve(e.target);
                    }
                }
            },
            { threshold: 0.12, rootMargin: "0px 0px -8% 0px" }
        );
        revealables.forEach((el) => io.observe(el));
    } else {
        revealables.forEach((el) => el.classList.add("in"));
    }

    /* -------------------------------------------------------- counting up -- */
    const counters = document.querySelectorAll("[data-count]");
    const runCounter = (el) => {
        const target = parseFloat(el.dataset.count);
        const suffix = el.dataset.suffix || "";
        if (target === 0) {
            el.textContent = "0" + suffix;
            return;
        }
        const dur = 1400;
        const start = performance.now();
        const step = (now) => {
            const t = Math.min((now - start) / dur, 1);
            // ease-out-expo: fast, then settles — reads as "counting up"
            const eased = t === 1 ? 1 : 1 - Math.pow(2, -10 * t);
            const value = Math.round(target * eased);
            el.textContent = value.toLocaleString("en-US") + suffix;
            if (t < 1) requestAnimationFrame(step);
        };
        requestAnimationFrame(step);
    };

    if ("IntersectionObserver" in window) {
        const co = new IntersectionObserver(
            (entries) => {
                for (const e of entries) {
                    if (e.isIntersecting) {
                        runCounter(e.target);
                        co.unobserve(e.target);
                    }
                }
            },
            { threshold: 0.5 }
        );
        counters.forEach((el) => co.observe(el));
    } else {
        counters.forEach(runCounter);
    }

    /* ------------------------------------------------- hero balance ticker -- */
    const ticker = document.getElementById("ticker");
    if (ticker && !reduced) {
        let value = 0;
        const target = 1872;
        const tick = () => {
            value += (target - value) * 0.06;
            const shown = value < target - 0.5 ? value : target;
            ticker.textContent = shown.toLocaleString("en-US", {
                minimumFractionDigits: 8,
                maximumFractionDigits: 8,
            });
            if (shown < target) requestAnimationFrame(tick);
        };
        setTimeout(() => requestAnimationFrame(tick), 700);
    } else if (ticker) {
        ticker.textContent = "1,872.00000000";
    }

    /* ------------------------------------------------ equation spotlight --- */
    const eq = document.getElementById("eq");
    if (eq && !reduced) {
        eq.addEventListener("pointermove", (ev) => {
            const r = eq.getBoundingClientRect();
            eq.style.setProperty("--mx", `${ev.clientX - r.left}px`);
            eq.style.setProperty("--my", `${ev.clientY - r.top}px`);
        });
    }

    /* ------------------------------------------------------ block conveyor -- */
    const chain = document.getElementById("chain");
    if (chain) {
        const makeBlock = (height, fresh) => {
            const el = document.createElement("div");
            el.className = "blk" + (fresh ? " new" : "");
            el.innerHTML = `<b>#${height}</b>6 NIGHT`;
            return el;
        };
        const link = () => {
            const el = document.createElement("div");
            el.className = "link";
            return el;
        };
        // Two identical halves so the -50% translate loops seamlessly.
        const build = (from) => {
            const frag = document.createDocumentFragment();
            for (let i = 0; i < 12; i++) {
                frag.appendChild(makeBlock(from + i, i === 11));
                frag.appendChild(link());
            }
            return frag;
        };
        chain.appendChild(build(1204));
        chain.appendChild(build(1204));
    }

    /* ------------------------------------------------------- mesh gradient -- */
    const canvas = document.getElementById("mesh");
    if (canvas && !reduced) {
        const ctx = canvas.getContext("2d");
        let w = 0;
        let h = 0;
        let raf = null;

        // Slow-drifting coloured orbs, blurred into each other. Cheap to draw
        // and it never repeats exactly.
        const orbs = [
            { x: 0.18, y: 0.28, r: 0.42, c: [124, 92, 255], sx: 0.00007, sy: 0.00005 },
            { x: 0.74, y: 0.22, r: 0.36, c: [184, 69, 216], sx: -0.00006, sy: 0.00008 },
            { x: 0.52, y: 0.72, r: 0.44, c: [53, 163, 196], sx: 0.00005, sy: -0.00006 },
            { x: 0.9, y: 0.62, r: 0.3, c: [86, 51, 196], sx: -0.00008, sy: -0.00004 },
        ];

        const resize = () => {
            const dpr = Math.min(window.devicePixelRatio || 1, 2);
            w = canvas.clientWidth;
            h = canvas.clientHeight;
            canvas.width = Math.floor(w * dpr);
            canvas.height = Math.floor(h * dpr);
            ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        };

        const draw = (t) => {
            ctx.clearRect(0, 0, w, h);
            ctx.globalCompositeOperation = "lighter";

            for (const o of orbs) {
                // Lissajous drift keeps them from ever lining up the same way.
                const cx = (o.x + Math.sin(t * o.sx) * 0.08) * w;
                const cy = (o.y + Math.cos(t * o.sy) * 0.08) * h;
                const rad = o.r * Math.max(w, h) * 0.75;

                const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, rad);
                const [r, gr, b] = o.c;
                g.addColorStop(0, `rgba(${r},${gr},${b},0.34)`);
                g.addColorStop(0.5, `rgba(${r},${gr},${b},0.11)`);
                g.addColorStop(1, `rgba(${r},${gr},${b},0)`);
                ctx.fillStyle = g;
                ctx.beginPath();
                ctx.arc(cx, cy, rad, 0, Math.PI * 2);
                ctx.fill();
            }

            ctx.globalCompositeOperation = "source-over";
            raf = requestAnimationFrame(draw);
        };

        resize();
        window.addEventListener("resize", resize);
        raf = requestAnimationFrame(draw);

        // Stop painting when the hero scrolls away — no reason to burn battery
        // animating something nobody is looking at.
        if ("IntersectionObserver" in window) {
            new IntersectionObserver(
                (entries) => {
                    const visible = entries[0].isIntersecting;
                    if (visible && raf === null) {
                        raf = requestAnimationFrame(draw);
                    } else if (!visible && raf !== null) {
                        cancelAnimationFrame(raf);
                        raf = null;
                    }
                },
                { threshold: 0 }
            ).observe(canvas);
        }
    }
})();
