#!/usr/bin/env bash
# Publish nightfallcoin.org from this workspace, and only if it checks out.
#
#     ./scripts/deploy-website.sh            check, then deploy
#     ./scripts/deploy-website.sh --check    check only, change nothing
#
# The site left the public repository on 8 Sep 2026 and now lives here, in the
# operator workspace, alone. That was a deliberate call and this script does
# not argue with it — but it did have a side effect worth naming: CI skips
# every website check via `hashFiles('website/wrangler.toml')`, and that file
# will never exist in the mirror again, so the condition is permanently false.
# Nine checks that had each caught a bug already shipped — dead download
# buttons in 0.8.3, README checksums two releases stale, an unreadable
# download button, cache busters frozen for a year — stopped running anywhere.
#
# They were not wrong. They were just left guarding a gate nobody walks
# through any more. This puts them back in front of the one that is used.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SITE="$ROOT/website"
CHECK_ONLY=0
[ "${1:-}" = "--check" ] && CHECK_ONLY=1

say()  { printf '\n\033[1m%s\033[0m\n' "$*"; }
step() { printf '    %s\n' "$*"; }
die()  { printf '\n\033[31m%s\033[0m\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------- the source ---
#
# Which tree gets published is the one thing that must not be guessed.
#
# The old routine was: copy the working tree into github/, deploy from
# github/website. That directory no longer holds a site at all. Running the
# old routine today would publish nothing, or — worse, if a stale copy is ever
# restored there — would replace the current site with an older one and look
# like a successful deploy while doing it.
#
# So the path is fixed to the workspace, and the mirror is refused outright
# rather than merely not chosen.
say "==> source"
[ -f "$SITE/wrangler.toml" ] || die "no wrangler.toml in $SITE — this is not the site tree"
[ -f "$SITE/public/index.html" ] || die "no public/index.html in $SITE"
if [ -e "$ROOT/github/website" ]; then
    die "github/website exists again. The site is published from the workspace,
not from the mirror. Deploying the mirror would overwrite the live site with
whatever is in there. Remove it, or fix this script on purpose."
fi
step "publishing from $SITE"

# ------------------------------------------------------- nothing may vanish ---
#
# The failure this guards against is not a broken page. It is a deploy that
# succeeds while quietly dropping pages that are live right now — which is
# exactly what publishing an older tree looks like from the outside.
#
# So every page the live site currently serves has to exist in the tree about
# to replace it. A page that is live and missing here is not an edit, it is a
# regression, and the script stops rather than asking.
say "==> nothing currently live may go missing"
LIVE_PAGES=(/ /chain/ /emission/ /build/ /audit/ /view-key/ /wallet/)
missing=0
for p in "${LIVE_PAGES[@]}"; do
    code="$(curl -fsS -o /dev/null -w '%{http_code}' --max-time 25 "https://nightfallcoin.org$p" 2>/dev/null || echo 000)"
    local_file="$SITE/public${p}index.html"
    [ "$p" = "/" ] && local_file="$SITE/public/index.html"
    if [ "$code" = "200" ] && [ ! -f "$local_file" ]; then
        printf '    \033[31mlive but not in this tree: %s\033[0m\n' "$p"
        missing=1
    fi
done
[ "$missing" = "0" ] || die "this tree is missing pages the site is serving. Refusing to publish
it — that is what replacing the new site with an old one looks like."
step "all ${#LIVE_PAGES[@]} live pages are present here"

# -------------------------------------------------------------- the checks ---
#
# Order is deliberate: the cheap structural ones first, so a typo is named in
# a second rather than after a minute of network calls.
say "==> checks"
CHECKS=(
    "check-site-design"
    "check-website-security"
    "check-chain-page"
    "check-button-colours"
    "check-web-wallet"
    "check-download-links"
    "check-docs-version"
    "check-bootstrap-deploy"
)
failed=0
for c in "${CHECKS[@]}"; do
    printf '    %-26s ' "$c"
    if out="$(node "$ROOT/scripts/$c.mjs" 2>&1)"; then
        echo "ok"
    else
        echo "FAILED"
        printf '%s\n' "$out" | sed 's/^/        /'
        failed=1
    fi
done

# Not in the loop: it takes an argument, and passing --check matters. Without
# it the script rewrites the busters instead of reporting on them, which would
# turn a check into a silent edit.
printf '    %-26s ' "stamp-cache-busters"
if out="$(node "$ROOT/scripts/stamp-cache-busters.mjs" --check 2>&1)"; then
    echo "ok"
else
    echo "FAILED"
    printf '%s\n' "$out" | sed 's/^/        /'
    echo "        fix with: node scripts/stamp-cache-busters.mjs"
    failed=1
fi

[ "$failed" = "0" ] || die "checks failed — nothing was published."

if [ "$CHECK_ONLY" = "1" ]; then
    say "checks passed. Nothing was published."
    exit 0
fi

# ------------------------------------------------------------------ publish ---
#
# wrangler uses its own stored login in ~/.wrangler. An earlier version of the
# deploy step passed CLOUDFLARE_API_TOKEN read from a file that does not exist
# on this machine: the variable came out empty, wrangler fell back to that
# login, and the deploy worked for a reason entirely unlike the one written
# down. Checked here for what it actually uses.
( cd "$SITE" && npx wrangler whoami >/dev/null 2>&1 ) \
    || die "wrangler is not logged in — run: cd website && npx wrangler login"

say "==> publishing"
( cd "$SITE" && npx wrangler deploy 2>&1 | tail -5 | sed 's/^/    /' )

# ------------------------------------------------------------------- verify ---
#
# Publishing and checking are not the same act, and only the second one is
# evidence. This reads the live site the way a stranger would and compares it
# to the bytes just sent.
say "==> reading the live site back"
sleep 4
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
curl -fsS --max-time 30 "https://nightfallcoin.org/?deploycheck=$RANDOM" -o "$tmp" \
    || die "the site did not answer after the deploy"

if cmp -s "$tmp" "$SITE/public/index.html"; then
    step "home page matches what was just published"
else
    # Not fatal on its own: the Worker may transform the HTML it serves. Say
    # so plainly instead of either ignoring it or crying wolf.
    step "home page differs from the local file — check whether the Worker"
    step "rewrites it, or whether an edge cache is still serving the old one."
fi

for p in "${LIVE_PAGES[@]}"; do
    code="$(curl -fsS -o /dev/null -w '%{http_code}' --max-time 25 "https://nightfallcoin.org$p" 2>/dev/null || echo 000)"
    [ "$code" = "200" ] || die "$p answers HTTP $code after the deploy"
done
step "all ${#LIVE_PAGES[@]} pages answer"

say "==> done"
