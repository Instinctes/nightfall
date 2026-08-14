#!/usr/bin/env bash
#
# Keep a Cloudflare A record pointed at this machine's public address.
#
# Written for a seed node on a home or office connection, where the ISP hands
# out an address that changes without warning. When it changes, the DNS name
# stops resolving to anything useful — harmless for nodes already connected,
# but new installs then find one seed instead of two, and nothing anywhere says
# so. This closes that gap.
#
#   ./cloudflare-ddns.sh                 update once
#   ./cloudflare-ddns.sh --install       install as a launchd agent (every 5 min)
#   ./cloudflare-ddns.sh --uninstall     remove it
#   ./cloudflare-ddns.sh --status        show what it would do, change nothing
#
# The API token is never passed on the command line and never printed. Create
# it yourself and put it in a file this script reads; see SETUP below.

set -euo pipefail

ZONE="nightfallcoin.org"
RECORD="seed2.nightfallcoin.org"
TOKEN_FILE="${NF_DDNS_TOKEN_FILE:-$HOME/.config/nightfall-ddns/token}"
STATE_FILE="$HOME/.config/nightfall-ddns/last-ip"
LOG="$HOME/Library/Logs/nightfall-ddns.log"
LABEL="org.nightfallcoin.ddns"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
INTERVAL=300

API="https://api.cloudflare.com/client/v4"

die() { printf '\033[31merror:\033[0m %s\n' "$1" >&2; exit 1; }
info() { printf '\033[36m==>\033[0m %s\n' "$1"; }
log() { printf '%s  %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$1" >> "$LOG"; }

setup_help() {
    cat <<'HELP'
SETUP — do this once, on the machine that runs the seed node.

1. Create a scoped API token at
   https://dash.cloudflare.com/profile/api-tokens

   Use "Create Custom Token" and give it exactly:

       Permissions   Zone · DNS · Edit
       Zone Resources  Include · Specific zone · nightfallcoin.org

   Nothing else. A token that can only edit DNS for one zone cannot touch
   your account, your Workers, or any other domain — so a machine sitting in
   an office cupboard holds the least authority that still does the job.

2. Store it, readable only by you:

       mkdir -p ~/.config/nightfall-ddns
       printf '%s' 'PASTE_TOKEN_HERE' > ~/.config/nightfall-ddns/token
       chmod 600 ~/.config/nightfall-ddns/token

   Use printf rather than echo so no trailing newline gets into the file, and
   mind that the command lands in your shell history — clear it afterwards if
   that bothers you.

3. Run ./cloudflare-ddns.sh --install
HELP
}

read_token() {
    [ -f "$TOKEN_FILE" ] || { setup_help; echo; die "no token at $TOKEN_FILE"; }
    local mode
    mode=$(stat -f '%OLp' "$TOKEN_FILE" 2>/dev/null || echo "")
    if [ -n "$mode" ] && [ "$mode" != "600" ]; then
        printf '\033[33mnote:\033[0m %s is mode %s — tightening to 600\n' "$TOKEN_FILE" "$mode"
        chmod 600 "$TOKEN_FILE"
    fi
    TOKEN=$(tr -d '\n\r ' < "$TOKEN_FILE")
    [ -n "$TOKEN" ] || die "token file is empty"
}

# Cloudflare's own view of our address. Using the same provider that serves the
# DNS avoids a class of mismatch where a third-party echo service reports a
# different egress address than the one Cloudflare would see.
public_ip() {
    curl -fsS --max-time 15 https://cloudflare.com/cdn-cgi/trace \
        | awk -F= '/^ip=/ {print $2}' \
        | tr -d '[:space:]'
}

cf() {
    curl -fsS --max-time 20 \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" "$@"
}

json_field() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)" 2>/dev/null; }

update() {
    local dry="${1:-no}"
    read_token
    mkdir -p "$(dirname "$STATE_FILE")" "$(dirname "$LOG")"

    local ip
    ip=$(public_ip) || die "could not determine the public address"
    [[ "$ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "got something that is not an IPv4 address: $ip"

    # Skip the API entirely when nothing changed. A record that is already
    # correct is the normal case, and there is no reason to call out every five
    # minutes to confirm it.
    if [ "$dry" = "no" ] && [ -f "$STATE_FILE" ] && [ "$(cat "$STATE_FILE")" = "$ip" ]; then
        log "unchanged $ip"
        exit 0
    fi

    local zone_id
    zone_id=$(cf "$API/zones?name=$ZONE" | json_field "d['result'][0]['id']") \
        || die "could not look up the zone — is the token scoped to $ZONE?"
    [ -n "${zone_id:-}" ] || die "zone $ZONE not found for this token"

    local rec
    rec=$(cf "$API/zones/$zone_id/dns_records?type=A&name=$RECORD")
    local rec_id current proxied
    rec_id=$(echo "$rec" | json_field "d['result'][0]['id'] if d['result'] else ''")
    current=$(echo "$rec" | json_field "d['result'][0]['content'] if d['result'] else ''")
    proxied=$(echo "$rec" | json_field "d['result'][0]['proxied'] if d['result'] else ''")

    if [ "$dry" = "yes" ]; then
        echo "record   $RECORD"
        echo "in DNS   ${current:-<missing>}   proxied=${proxied:-?}"
        echo "actual   $ip"
        if [ "$current" = "$ip" ] && [ "$proxied" = "False" ]; then
            echo "→ nothing to do"
        else
            echo "→ would update"
        fi
        exit 0
    fi

    local body
    body=$(printf '{"type":"A","name":"%s","content":"%s","ttl":60,"proxied":false}' "$RECORD" "$ip")

    if [ -z "$rec_id" ]; then
        cf -X POST "$API/zones/$zone_id/dns_records" --data "$body" > /dev/null \
            || die "could not create $RECORD"
        log "created $RECORD -> $ip"
        info "created $RECORD -> $ip"
    elif [ "$current" != "$ip" ] || [ "$proxied" != "False" ]; then
        # proxied is forced back to false on every write. Cloudflare's proxy
        # only carries HTTP; with it on, the name resolves to Cloudflare's
        # addresses and nobody reaches port 17891 — the record would look
        # perfectly healthy and be useless.
        cf -X PATCH "$API/zones/$zone_id/dns_records/$rec_id" --data "$body" > /dev/null \
            || die "could not update $RECORD"
        log "updated $RECORD $current -> $ip"
        info "updated $RECORD $current -> $ip"
    else
        log "already correct $ip"
    fi

    printf '%s' "$ip" > "$STATE_FILE"
}

install_agent() {
    read_token
    local self
    self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
    mkdir -p "$(dirname "$PLIST")" "$(dirname "$LOG")"

    cat > "$PLIST" <<PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>$LABEL</string>
    <key>ProgramArguments</key>
    <array><string>$self</string></array>
    <key>StartInterval</key><integer>$INTERVAL</integer>
    <key>RunAtLoad</key><true/>
    <key>StandardOutPath</key><string>$LOG</string>
    <key>StandardErrorPath</key><string>$LOG</string>
    <key>ProcessType</key><string>Background</string>
</dict>
</plist>
PLIST_EOF

    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$PLIST"
    info "installed — checks every $((INTERVAL / 60)) minutes"
    info "log: $LOG"
}

uninstall_agent() {
    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
    rm -f "$PLIST"
    info "removed"
}

case "${1:-}" in
    --install)   install_agent ;;
    --uninstall) uninstall_agent ;;
    --status)    update yes ;;
    --setup)     setup_help ;;
    "")          update no ;;
    *)           die "unknown option: $1 (try --install, --uninstall, --status, --setup)" ;;
esac
