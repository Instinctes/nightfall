#!/usr/bin/env python3
"""Read-only audit of the NIGHTFALL Discord server.

Answers the questions a screenshot cannot: who may post where, which roles
carry dangerous permissions, whether AutoMod exists, whether onboarding is
configured, and which channels are empty shelves.

This script NEVER writes to Discord. It issues GET requests only. There is a
companion script for changes, and it asks before each one.

    ./scripts/discord-audit.py                 # full report
    ./scripts/discord-audit.py --json          # machine-readable

The bot token is read from a file and never printed, never logged, never
passed on a command line where `ps` would show it:

    ~/.config/nightfall/discord-bot-token      # chmod 600

Setup, once:

  1. https://discord.com/developers/applications → New Application
  2. Bot → Reset Token → copy it
  3. mkdir -p ~/.config/nightfall
     printf '%s' 'PASTE_TOKEN_HERE' > ~/.config/nightfall/discord-bot-token
     chmod 600 ~/.config/nightfall/discord-bot-token
  4. OAuth2 → URL Generator → scope `bot`, permission `Administrator`
     (or at minimum: View Channels, Read Message History, Manage Server)
     → open the URL → add to NIGHTFALL
  5. Server Settings → Enable Developer Mode, right-click the server icon →
     Copy Server ID → put it in ~/.config/nightfall/discord-guild-id

Nothing here needs your Discord password, and the token stays in a file only
you can read.
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.request

API = "https://discord.com/api/v10"
CFG = os.path.expanduser("~/.config/nightfall")
TOKEN_FILE = os.path.join(CFG, "discord-bot-token")
GUILD_FILE = os.path.join(CFG, "discord-guild-id")

# Permission bits worth naming. Discord returns a bitfield as a string.
PERMS = {
    "ADMINISTRATOR": 1 << 3,
    "MANAGE_CHANNELS": 1 << 4,
    "MANAGE_GUILD": 1 << 5,
    "MANAGE_MESSAGES": 1 << 13,
    "MENTION_EVERYONE": 1 << 17,
    "KICK_MEMBERS": 1 << 1,
    "BAN_MEMBERS": 1 << 2,
    "MANAGE_ROLES": 1 << 28,
    "MANAGE_WEBHOOKS": 1 << 29,
    "SEND_MESSAGES": 1 << 11,
    "VIEW_CHANNEL": 1 << 10,
    "CREATE_INSTANT_INVITE": 1 << 0,
    "ATTACH_FILES": 1 << 15,
    "EMBED_LINKS": 1 << 14,
    "ADD_REACTIONS": 1 << 6,
}

# The ones that let someone wreck or impersonate. A public community server
# should be able to name every holder of these without thinking.
DANGEROUS = [
    "ADMINISTRATOR",
    "MANAGE_GUILD",
    "MANAGE_CHANNELS",
    "MANAGE_ROLES",
    "MANAGE_WEBHOOKS",
    "MENTION_EVERYONE",
    "BAN_MEMBERS",
]

CHANNEL_TYPES = {
    0: "text",
    2: "voice",
    4: "category",
    5: "announcement",
    13: "stage",
    15: "forum",
    16: "media",
}

SYSTEM_FLAGS = {
    "SUPPRESS_JOIN_NOTIFICATIONS": 1 << 0,
    "SUPPRESS_PREMIUM_SUBSCRIPTIONS": 1 << 1,
    "SUPPRESS_GUILD_REMINDER_NOTIFICATIONS": 1 << 2,
    "SUPPRESS_JOIN_NOTIFICATION_REPLIES": 1 << 3,
}


def read_secret(path: str, what: str) -> str:
    if not os.path.exists(path):
        sys.exit(
            f"missing {path}\n"
            f"  {what}\n"
            f"  See the header of this file for the one-time setup."
        )
    mode = os.stat(path).st_mode & 0o777
    if mode & 0o077:
        sys.exit(
            f"{path} is mode {mode:o} — readable by others.\n"
            f"  chmod 600 {path}"
        )
    with open(path, encoding="utf-8") as fh:
        value = fh.read().strip()
    if not value:
        sys.exit(f"{path} is empty")
    return value


def get(token: str, path: str, *, allow_fail: bool = False):
    """One GET, with a single retry on Discord's rate limit."""
    for attempt in (1, 2):
        req = urllib.request.Request(
            API + path,
            headers={
                "Authorization": f"Bot {token}",
                "User-Agent": "NightfallAudit/1.0 (+https://nightfallcoin.org)",
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                return json.loads(resp.read().decode("utf-8"))
        except urllib.error.HTTPError as e:
            if e.code == 429 and attempt == 1:
                body = json.loads(e.read().decode("utf-8") or "{}")
                time.sleep(float(body.get("retry_after", 1.0)) + 0.25)
                continue
            if allow_fail:
                return {"__error__": f"HTTP {e.code}"}
            detail = ""
            try:
                detail = e.read().decode("utf-8")[:200]
            except Exception:
                pass
            sys.exit(f"GET {path} failed: HTTP {e.code} {detail}")
        except urllib.error.URLError as e:
            sys.exit(f"GET {path} failed: {e.reason}")
    return {}


def bits(value) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def named_perms(value, names) -> list[str]:
    n = bits(value)
    return [p for p in names if n & PERMS[p]]


def collect(token: str, guild_id: str) -> dict:
    guild = get(token, f"/guilds/{guild_id}?with_counts=true")
    channels = get(token, f"/guilds/{guild_id}/channels")
    roles = guild.get("roles", [])

    # These need MANAGE_GUILD; degrade rather than die.
    onboarding = get(token, f"/guilds/{guild_id}/onboarding", allow_fail=True)
    automod = get(token, f"/guilds/{guild_id}/auto-moderation/rules", allow_fail=True)
    invites = get(token, f"/guilds/{guild_id}/invites", allow_fail=True)
    webhooks = get(token, f"/guilds/{guild_id}/webhooks", allow_fail=True)

    # Per-channel detail: last message and pin count. One request each; a
    # server this size is a handful of calls, so no pagination games.
    detail = {}
    for ch in channels:
        if ch.get("type") not in (0, 5, 15, 16):
            continue
        cid = ch["id"]
        msgs = get(token, f"/channels/{cid}/messages?limit=1", allow_fail=True)
        pins = get(token, f"/channels/{cid}/pins", allow_fail=True)
        detail[cid] = {
            "empty": isinstance(msgs, list) and len(msgs) == 0,
            "unreadable": isinstance(msgs, dict),
            "last": (msgs[0].get("timestamp") if isinstance(msgs, list) and msgs else None),
            "pins": len(pins) if isinstance(pins, list) else None,
        }
        time.sleep(0.12)  # stay well under the rate limit

    return {
        "guild": guild,
        "channels": channels,
        "roles": roles,
        "onboarding": onboarding,
        "automod": automod,
        "invites": invites,
        "webhooks": webhooks,
        "detail": detail,
    }


def report(d: dict) -> list[str]:
    """Returns the findings. Each line is something to decide about."""
    g = d["guild"]
    channels = d["channels"]
    roles = d["roles"]
    out: list[str] = []
    findings: list[str] = []

    cats = {c["id"]: c["name"] for c in channels if c.get("type") == 4}
    everyone = next((r for r in roles if r.get("name") == "@everyone"), None)

    out.append("═══ SERVER ═══")
    out.append(f"  Name:            {g.get('name')}")
    out.append(f"  Mitglieder:      {g.get('approximate_member_count')} "
               f"({g.get('approximate_presence_count')} online)")
    out.append(f"  Boost-Stufe:     {g.get('premium_tier')} "
               f"({g.get('premium_subscription_count', 0)} Boosts)")
    out.append(f"  Verifizierung:   {g.get('verification_level')} (0=keine … 4=höchste)")
    out.append(f"  Inhaltsfilter:   {g.get('explicit_content_filter')} (0=aus, 2=alle)")
    out.append(f"  MFA für Mods:    {'ja' if g.get('mfa_level') else 'NEIN'}")
    out.append(f"  Features:        {', '.join(sorted(g.get('features', []))) or '—'}")
    out.append(f"  Regelkanal:      {g.get('rules_channel_id') or 'NICHT GESETZT'}")
    out.append(f"  Systemkanal:     {g.get('system_channel_id') or '—'}")

    flags = bits(g.get("system_channel_flags"))
    on = [n for n, b in SYSTEM_FLAGS.items() if not (flags & b)]
    out.append(f"  Systemnachrichten aktiv: {', '.join(on) or 'keine'}")

    # --- Findings ---------------------------------------------------------
    if not (flags & SYSTEM_FLAGS["SUPPRESS_JOIN_NOTIFICATIONS"]):
        findings.append(
            "Beitrittsmeldungen sind AN. Das ist der Grund, warum #welcome nur aus "
            "'X ist beigetreten' besteht. Servereinstellungen → Übersicht → "
            "Systemnachrichten."
        )
    if not (flags & SYSTEM_FLAGS["SUPPRESS_JOIN_NOTIFICATION_REPLIES"]):
        findings.append(
            "'Winke, um Hallo zu sagen'-Sticker sind AN. Dieselbe Stelle, zweiter Haken."
        )
    if int(g.get("verification_level", 0)) < 2:
        findings.append(
            f"Verifizierungsstufe {g.get('verification_level')}. Für einen "
            "Krypto-Server zu niedrig — Stufe 2 ('verifizierte E-Mail und "
            "5 Minuten Mitglied') hält Wegwerf-Accounts von Scam-Wellen ab."
        )
    if not g.get("mfa_level"):
        findings.append(
            "Zwei-Faktor für Moderatoren ist AUS. Wer einen Mod-Account übernimmt, "
            "kann posten, als wärst du es. Bei einem Coin-Projekt ist das der "
            "wahrscheinlichste Angriff überhaupt."
        )
    if not g.get("rules_channel_id"):
        findings.append("Kein Regelkanal gesetzt (Community-Einstellungen).")

    # --- Kanäle -----------------------------------------------------------
    out.append("")
    out.append("═══ KANÄLE ═══")
    by_name: dict[str, list[str]] = {}
    for ch in sorted(channels, key=lambda c: (c.get("position", 0), c.get("name", ""))):
        t = ch.get("type")
        if t == 4:
            continue
        name = ch.get("name", "?")
        by_name.setdefault(name, []).append(CHANNEL_TYPES.get(t, str(t)))
        cat = cats.get(ch.get("parent_id"), "—")
        det = d["detail"].get(ch["id"], {})

        # Can @everyone post here?
        deny = allow = 0
        for ow in ch.get("permission_overwrites", []):
            if everyone and ow.get("id") == everyone["id"]:
                deny, allow = bits(ow.get("deny")), bits(ow.get("allow"))
        base_send = bits(everyone.get("permissions") if everyone else 0) & PERMS["SEND_MESSAGES"]
        can_send = bool((base_send or allow & PERMS["SEND_MESSAGES"])
                        and not deny & PERMS["SEND_MESSAGES"])
        readonly = "" if can_send else "  [schreibgeschützt]"

        state = ""
        if det.get("unreadable"):
            state = "  [Bot darf nicht lesen]"
        elif det.get("empty"):
            state = "  [LEER]"
        elif det.get("pins") == 0:
            state = "  [nichts angeheftet]"

        out.append(f"  {cat:<16} #{name:<20} {CHANNEL_TYPES.get(t, t):<12}"
                   f"{readonly}{state}")

        topic = (ch.get("topic") or "").strip()
        if t in (0, 5) and not topic:
            findings.append(f"#{name} hat kein Kanal-Thema.")
        elif len(topic) > 100:
            findings.append(
                f"#{name}: Thema ist {len(topic)} Zeichen und wird in der "
                "Kopfzeile abgeschnitten. Ein Satz gehört hin, der Rest in "
                "einen angehefteten Beitrag."
            )

    for name, kinds in by_name.items():
        if len(kinds) > 1:
            findings.append(
                f"ZWEI Kanäle heißen #{name} ({', '.join(kinds)}). Sieht nach "
                "Versehen aus und niemand weiß, welchem er folgen soll."
            )

    for ch in channels:
        det = d["detail"].get(ch.get("id"), {})
        if det.get("empty"):
            findings.append(
                f"#{ch.get('name')} ist vollständig leer. Ein leerer Kanal lässt "
                "den Server toter wirken, als er ist — füllen oder löschen."
            )
        elif det.get("pins") == 0 and ch.get("name") in (
            "start-here", "rules", "links", "welcome", "faq", "help"
        ):
            findings.append(
                f"#{ch.get('name')} hat keinen angehefteten Beitrag, obwohl genau "
                "das seine Aufgabe wäre."
            )

    # --- Rollen -----------------------------------------------------------
    out.append("")
    out.append("═══ ROLLEN ═══")
    for r in sorted(roles, key=lambda r: -r.get("position", 0)):
        dang = named_perms(r.get("permissions"), DANGEROUS)
        colour = f"#{r.get('color'):06x}" if r.get("color") else "keine Farbe"
        out.append(f"  {r.get('name'):<22} {colour:<12} "
                   f"{'sichtbar' if r.get('hoist') else '        '}  "
                   f"{', '.join(dang) or '—'}")
        if "ADMINISTRATOR" in dang and r.get("name") != "@everyone":
            findings.append(
                f"Rolle '{r.get('name')}' hat ADMINISTRATOR. Das ist jedes Recht "
                "auf einmal, inklusive Server löschen. Einzelrechte vergeben."
            )

    if everyone:
        bad = named_perms(everyone.get("permissions"), DANGEROUS)
        if bad:
            findings.append(
                f"@everyone hat {', '.join(bad)} — das gilt für JEDEN, der "
                "beitritt, auch für einen Bot mit gestohlenem Account."
            )

    # --- AutoMod, Onboarding, Einladungen, Webhooks ------------------------
    out.append("")
    out.append("═══ SCHUTZ UND ONBOARDING ═══")
    am = d["automod"]
    if isinstance(am, list):
        out.append(f"  AutoMod-Regeln:  {len(am)}")
        for r in am:
            out.append(f"    - {r.get('name')} (aktiviert: {r.get('enabled')})")
        if not am:
            findings.append(
                "KEINE AutoMod-Regel. Ein Krypto-Server ohne Spam- und "
                "Link-Filter wird früher oder später zur Phishing-Bühne: "
                "jemand kopiert deinen Namen, postet eine falsche Wallet-URL, "
                "und deine Mitglieder verlieren Coins. Discord bringt fertige "
                "Regeln mit (Spam, erwähnungs-Spam, eigene Wortliste)."
            )
    else:
        out.append("  AutoMod:         nicht lesbar (Bot braucht MANAGE_GUILD)")

    ob = d["onboarding"]
    if isinstance(ob, dict) and "__error__" not in ob:
        enabled = ob.get("enabled")
        out.append(f"  Onboarding:      {'AN' if enabled else 'AUS'}")
        out.append(f"  Prompts:         {len(ob.get('prompts', []))}")
        out.append(f"  Standardkanäle:  {len(ob.get('default_channel_ids', []))}")
        if not enabled:
            findings.append(
                "Onboarding ist AUS. Damit landet jeder Neue in derselben "
                "Kanalflut. Angeschaltet wählt er beim Eintritt 'ich mine' / "
                "'ich betreibe eine Node' / 'ich schaue zu' und sieht nur das, "
                "was ihn angeht."
            )
    else:
        out.append("  Onboarding:      nicht lesbar (Bot braucht MANAGE_GUILD)")

    inv = d["invites"]
    if isinstance(inv, list):
        out.append(f"  Einladungen:     {len(inv)}")
        for i in inv:
            exp = i.get("expires_at") or "läuft nie ab"
            out.append(f"    - {i.get('code')} → #{(i.get('channel') or {}).get('name')}"
                       f"  {i.get('uses')} Nutzungen, {exp}")
        stale = [i for i in inv if i.get("expires_at")]
        if stale:
            findings.append(
                f"{len(stale)} Einladung(en) laufen ab. Der Link auf Website und "
                "GitHub muss ein Dauerlink sein, sonst zeigt er irgendwann ins Leere."
            )
    wh = d["webhooks"]
    if isinstance(wh, list):
        out.append(f"  Webhooks:        {len(wh)}")
        for w in wh:
            out.append(f"    - {w.get('name')} → Kanal {w.get('channel_id')}")

    return out, findings


def main() -> None:
    token = read_secret(TOKEN_FILE, "Bot-Token")
    guild = read_secret(GUILD_FILE, "Server-ID")

    me = get(token, "/users/@me")
    print(f"Bot: {me.get('username')}#{me.get('discriminator')} "
          f"(nur Leserechte werden benutzt)\n")

    data = collect(token, guild)

    if "--json" in sys.argv:
        # Token taucht hier nirgends auf; das ist reine Serverstruktur.
        print(json.dumps(data, indent=2, ensure_ascii=False))
        return

    lines, findings = report(data)
    print("\n".join(lines))
    print("")
    print("═══ BEFUNDE ═══")
    if not findings:
        print("  keine")
    for i, f in enumerate(findings, 1):
        print(f"  {i:>2}. {f}")
    print(f"\n  {len(findings)} Punkt(e). Nichts davon wurde verändert — "
          f"dieses Skript schreibt nicht.")


if __name__ == "__main__":
    main()
