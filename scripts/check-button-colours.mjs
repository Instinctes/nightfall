#!/usr/bin/env node
/* A container's link colour must not repaint a button.
 *
 * `.btn-primary` is one class, specificity (0,1,0). Any rule of the shape
 * `.container a` is (0,1,1) and beats it, so a button placed inside that
 * container loses its white lettering and takes the container's link colour
 * instead — on a button that is filled with a bright gradient, that is text
 * you cannot read.
 *
 * This was not one mistake. Four rules had it independently: .nav-links a,
 * .doc a, .foot-col a and .upfront-foot a. The nav one repainted the site's
 * main Download button in --text-dim on every page, and it had been that way
 * long enough that nobody saw it any more — it still looks like a button, so
 * the eye fills in the label it expects. It took someone saying the text was
 * hard to read.
 *
 * The rule is not "never write `.container a`". It is "say whether you mean
 * buttons", which `:not(.btn)` does in four characters.
 */
import { readFileSync } from "node:fs";

const ROOT = new URL("../", import.meta.url).pathname;
const css = readFileSync(ROOT + "website/public/css/style.css", "utf8");

const problems = [];

for (const m of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const body = m[2];
    // `color:` and not `border-color:` / `background-color:` etc.
    if (!/(?<![-\w])color\s*:/.test(body)) continue;

    for (const raw of m[1].split(",")) {
        const sel = raw.trim();
        // A descendant `a` under something else — the shape that outranks a
        // single class. A bare `a` is fine: it loses to .btn-primary.
        if (!/\ba\s*$/.test(sel) || sel === "a") continue;
        if (sel.includes(":not(.btn)")) continue;

        const colour = (body.match(/(?<![-\w])color\s*:\s*([^;]+)/) || [])[1] || "?";
        problems.push(
            `${sel} sets color: ${colour.trim()} and does not exclude .btn — ` +
                `a button inside it loses .btn-primary's white text`,
        );
    }
}

if (problems.length) {
    console.error("button colours:");
    for (const p of problems) console.error("  - " + p);
    console.error("\nadd :not(.btn) to the selector, as .nav-links a and .doc a do");
    process.exit(1);
}
console.log("button colours ok — no container link rule outranks .btn-primary");
