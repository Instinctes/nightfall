/* Mobile navigation drawer.
 *
 * Below 820px the stylesheet hides every text link in the bar, which left a
 * phone with a logo, two icons and a Download button — no way to reach
 * Network, Audit, Emission or anything else. This adds the button that opens
 * them.
 *
 * The drawer is *built from the bar* rather than written a second time in
 * every page's HTML. Five pages each carry their own copy of the nav; a
 * hand-written second list would have gone stale the first time a link was
 * added to one of them and not the others. Clone what is already there and
 * the two cannot disagree.
 *
 * No dependencies, no framework, and it does nothing at all if the markup it
 * expects is missing.
 */
(function () {
    "use strict";

    var nav = document.getElementById("nav");
    if (!nav) return;
    var bar = nav.querySelector(".nav-links");
    if (!bar) return;

    var OPEN = "menu-open";
    var WIDE = window.matchMedia("(min-width: 821px)");

    /* ---------------------------------------------------------- drawer -- */

    var drawer = document.createElement("div");
    drawer.className = "nav-drawer";
    drawer.id = "nav-drawer";
    drawer.hidden = true;

    var list = document.createElement("div");
    list.className = "nav-drawer-links";
    drawer.appendChild(list);

    Array.prototype.forEach.call(bar.children, function (el) {
        if (el.tagName === "A" && !el.classList.contains("nav-icon")) {
            var a = el.cloneNode(true);
            // The Download button keeps its emphasis, everything else is a row.
            if (!a.classList.contains("btn")) a.className = "nav-drawer-link";
            else a.className = "btn btn-primary nav-drawer-cta";
            list.appendChild(a);
        }
    });

    var social = bar.querySelector(".nav-social");
    if (social) {
        var row = social.cloneNode(true);
        row.className = "nav-drawer-social";
        drawer.appendChild(row);
    }

    nav.appendChild(drawer);

    /* ---------------------------------------------------------- button -- */

    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "nav-burger";
    btn.setAttribute("aria-label", "Open menu");
    btn.setAttribute("aria-expanded", "false");
    btn.setAttribute("aria-controls", "nav-drawer");
    btn.innerHTML = "<span></span><span></span><span></span>";
    bar.appendChild(btn);

    /* ----------------------------------------------------------- state -- */

    function setOpen(open) {
        if (open) drawer.hidden = false;
        nav.classList.toggle(OPEN, open);
        btn.setAttribute("aria-expanded", open ? "true" : "false");
        btn.setAttribute("aria-label", open ? "Close menu" : "Open menu");
        // Keep it out of the tab order while it is closed, but only after the
        // slide-out has finished, or the transition never runs.
        if (!open) {
            window.setTimeout(function () {
                if (!nav.classList.contains(OPEN)) drawer.hidden = true;
            }, 220);
        }
    }

    btn.addEventListener("click", function (e) {
        e.stopPropagation();
        setOpen(!nav.classList.contains(OPEN));
    });

    // Any link closes it. Same-page anchors would otherwise leave the drawer
    // covering the section it just scrolled to.
    drawer.addEventListener("click", function (e) {
        if (e.target.closest("a")) setOpen(false);
    });

    document.addEventListener("click", function (e) {
        if (!nav.classList.contains(OPEN)) return;
        if (!nav.contains(e.target)) setOpen(false);
    });

    document.addEventListener("keydown", function (e) {
        if (e.key === "Escape" && nav.classList.contains(OPEN)) {
            setOpen(false);
            btn.focus();
        }
    });

    // Rotating a phone into landscape can cross the breakpoint. Leaving the
    // drawer open there stacks it on top of a bar that already shows the links.
    function onWide(e) {
        if (e.matches) setOpen(false);
    }
    if (WIDE.addEventListener) WIDE.addEventListener("change", onWide);
    else if (WIDE.addListener) WIDE.addListener(onWide);
})();
