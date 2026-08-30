#!/usr/bin/env python3
"""One-shot setup for the NIGHTFALL Discord guild.

Idempotent: matching names are updated, not duplicated.
Needs a bot with Administrator on guild 1539877192622678037.

    DISCORD_BOT_TOKEN=... python3 scripts/setup-discord.py
"""

from __future__ import annotations

import base64
import json
import os
import ssl
import sys
import time
import urllib.error
import urllib.request
from typing import Any

try:
    import certifi

    SSL_CTX = ssl.create_default_context(cafile=certifi.where())
except Exception:
    SSL_CTX = ssl.create_default_context()

API = "https://discord.com/api/v10"
GUILD_ID = "1539877192622678037"
KNOWN_GENERAL = "1539877193335963670"
ICON_PATH = os.path.join(
    os.path.dirname(__file__),
    "..",
    "website",
    "public",
    "assets",
    "logo-256.png",
)

# Permission bits (Discord docs).
P_CREATE_INVITE = 1 << 0
P_ADD_REACTIONS = 1 << 6
P_VIEW_CHANNEL = 1 << 10
P_SEND_MESSAGES = 1 << 11
P_SEND_TTS = 1 << 12
P_MANAGE_MESSAGES = 1 << 13
P_EMBED_LINKS = 1 << 14
P_ATTACH_FILES = 1 << 15
P_READ_HISTORY = 1 << 16
P_MENTION_EVERYONE = 1 << 17
P_EXT_EMOJI = 1 << 18
P_CONNECT = 1 << 20
P_SPEAK = 1 << 21
P_USE_VAD = 1 << 25
P_CHANGE_NICK = 1 << 26
P_USE_APP_COMMANDS = 1 << 31
P_CREATE_PUBLIC_THREADS = 1 << 35
P_CREATE_PRIVATE_THREADS = 1 << 36
P_SEND_IN_THREADS = 1 << 38
P_USE_SOUNDBOARD = 1 << 42
P_SEND_VOICE_MESSAGES = 1 << 46
P_SEND_POLLS = 1 << 49

EVERYONE_ALLOW = (
    P_CREATE_INVITE
    | P_ADD_REACTIONS
    | P_VIEW_CHANNEL
    | P_SEND_MESSAGES
    | P_EMBED_LINKS
    | P_ATTACH_FILES
    | P_READ_HISTORY
    | P_EXT_EMOJI
    | P_CONNECT
    | P_SPEAK
    | P_USE_VAD
    | P_CHANGE_NICK
    | P_USE_APP_COMMANDS
    | P_CREATE_PUBLIC_THREADS
    | P_SEND_IN_THREADS
    | P_USE_SOUNDBOARD
    | P_SEND_VOICE_MESSAGES
)
EVERYONE_DENY = P_MENTION_EVERYONE | P_SEND_TTS | P_CREATE_PRIVATE_THREADS

INFO_DENY_SEND = (
    P_SEND_MESSAGES
    | P_SEND_TTS
    | P_CREATE_PUBLIC_THREADS
    | P_CREATE_PRIVATE_THREADS
    | P_SEND_IN_THREADS
    | P_SEND_POLLS
)

STAFF_ALLOW = (
    P_VIEW_CHANNEL
    | P_SEND_MESSAGES
    | P_MANAGE_MESSAGES
    | P_EMBED_LINKS
    | P_ATTACH_FILES
    | P_READ_HISTORY
    | P_MENTION_EVERYONE
    | P_CONNECT
    | P_SPEAK
)

# Brand colours (decimal).
C_VIOLET = 0x7C5CFF
C_MAGENTA = 0xB845D8
C_TEAL = 0x35A3C4
C_GOLD = 0xFFC85C
C_GREEN = 0x4AE0A8
C_DIM = 0xA89FC6

ROLES = [
    # hoist, mentionable, name, colour, perms extra (on top of @everyone)
    {
        "name": "Founder",
        "color": C_VIOLET,
        "hoist": True,
        "mentionable": False,
        "permissions": str(EVERYONE_ALLOW | STAFF_ALLOW | P_MENTION_EVERYONE),
    },
    {
        "name": "Moderator",
        "color": C_MAGENTA,
        "hoist": True,
        "mentionable": True,
        "permissions": str(EVERYONE_ALLOW | STAFF_ALLOW),
    },
    {
        "name": "Contributor",
        "color": C_TEAL,
        "hoist": True,
        "mentionable": False,
        "permissions": str(EVERYONE_ALLOW),
    },
    {
        "name": "Miner",
        "color": C_GOLD,
        "hoist": False,
        "mentionable": True,
        "permissions": str(EVERYONE_ALLOW),
    },
    {
        "name": "Node",
        "color": C_GREEN,
        "hoist": False,
        "mentionable": True,
        "permissions": str(EVERYONE_ALLOW),
    },
    {
        "name": "Member",
        "color": C_DIM,
        "hoist": False,
        "mentionable": False,
        "permissions": str(EVERYONE_ALLOW),
    },
]

# Categories in display order. Channels listed under each.
# kind: text | news | voice
LAYOUT: list[dict[str, Any]] = [
    {
        "name": "INFO",
        "channels": [
            {
                "name": "welcome",
                "kind": "text",
                "readonly": True,
                "topic": (
                    "Start here. Then #rules and #start-here. "
                    "Website nightfallcoin.org · source github.com/Instinctes/nightfall. "
                    "Not 100% anonymous. No official price. Staff never DM first."
                ),
                "pin": "welcome",
            },
            {
                "name": "rules",
                "kind": "text",
                "readonly": True,
                "topic": (
                    "House rules. Never paste a seed. No price talk. "
                    "English in #general, German in #deutsch. Staff never DM you first."
                ),
                "pin": "rules",
            },
            {
                "name": "announcements",
                "kind": "news",
                "readonly": True,
                "topic": (
                    "Official only — releases, chain notices, seed status. "
                    "If it is not in this channel, it is not an official statement."
                ),
                "pin": "announcements",
            },
            {
                "name": "releases",
                "kind": "text",
                "readonly": True,
                "topic": (
                    "Desktop Core 0.8.1 · phone/web 0.7.0. Same chain: "
                    "genesis 061a052d… · protocol v8 · wire v6 · magic NFL2. "
                    "Verify SHA256SUMS next to the download."
                ),
                "pin": "releases",
            },
            {
                "name": "start-here",
                "kind": "text",
                "readonly": True,
                "topic": (
                    "Wallet, mining, checksums. https://nightfallcoin.org/#download "
                    "Unsigned builds. Windows: allow port 17891 or you sit on block 1."
                ),
                "pin": "start-here",
            },
            {
                "name": "links",
                "kind": "text",
                "readonly": True,
                "topic": "Website, GitHub, Bitcointalk, network numbers. Nothing else is official.",
                "pin": "links",
            },
        ],
    },
    {
        "name": "COMMUNITY",
        "channels": [
            {
                "name": "general",
                "kind": "text",
                "alias": ["allgemein", "general", "lounge"],
                "topic": (
                    "Main room. English. Questions, mining, protocol. "
                    "Keep seeds off Discord. No price. German → #deutsch."
                ),
            },
            {
                "name": "deutsch",
                "kind": "text",
                "topic": (
                    "Deutscher Raum. Dieselbe Kette, dieselben Regeln: "
                    "kein Seed, kein Preis, nicht „100 % anonym“."
                ),
                "pin": "deutsch",
            },
            {
                "name": "off-topic",
                "kind": "text",
                "topic": "Anything that is not Nightfall. Still no scams, still no seeds.",
            },
        ],
    },
    {
        "name": "MINING & NODES",
        "channels": [
            {
                "name": "mining",
                "kind": "text",
                "topic": (
                    "Nighthash-v2 = Argon2id 32 MiB · ~15 s blocks · no pool. "
                    "Connect to a peer before you mine. Dashboard hash = difficulty ÷ 15, "
                    "an estimate, not a miner census."
                ),
                "pin": "mining",
            },
            {
                "name": "nodes",
                "kind": "text",
                "topic": (
                    "nightfalld · P2P 17891 · RPC 127.0.0.1:17881 (newline JSON, method status). "
                    "Seed seed.nightfallcoin.org. Public numbers: nightfallcoin.org/emission/"
                ),
                "pin": "nodes",
            },
        ],
    },
    {
        "name": "SUPPORT",
        "channels": [
            {
                "name": "help",
                "kind": "text",
                "topic": (
                    "Wallet / sync / Tor. Say OS + version (Core 0.8.1, phone 0.7.0). "
                    "Windows on BLOCKS = 1 is almost always port 17891 / Defender."
                ),
                "pin": "help",
            },
            {
                "name": "bugs",
                "kind": "text",
                "topic": (
                    "Crashes and wrong numbers. Version, OS, steps, what you expected. "
                    "Never paste a seed. One report per message."
                ),
                "pin": "bugs",
            },
        ],
    },
    {
        "name": "DEVELOPMENT",
        "channels": [
            {
                "name": "dev",
                "kind": "text",
                "topic": (
                    "Protocol v8 · wire v6 · magic NFL2 · genesis 061a052d… "
                    "Spec and PRs on github.com/Instinctes/nightfall. "
                    "exploit_regression.rs is not optional."
                ),
                "pin": "dev",
            },
        ],
    },
    {
        "name": "STAFF",
        "staff_only": True,
        "channels": [
            {
                "name": "staff",
                "kind": "text",
                "topic": "Moderators and founder. Not visible to the room.",
            },
            {
                "name": "mod-log",
                "kind": "text",
                "topic": "Discord community updates and a quiet log. Keep it empty of chatter.",
            },
        ],
    },
    {
        "name": "VOICE",
        "channels": [
            {
                "name": "Lounge",
                "kind": "voice",
                "alias": ["allgemein"],
                "topic": "Talk.",
            },
            {
                "name": "Mining",
                "kind": "voice",
                "topic": "Leave it on while you hash.",
            },
        ],
    },
]

PINS: dict[str, str] = {
    "welcome": """**NIGHTFALL** — money that refuses to snitch.

A privacy Layer-1. Amounts stay hidden. Addresses never appear on the chain. The supply is a proof every node re-checks, not a promise.

This is **not** “100% anonymous.” The graph is mixed inside a block, not erased. Tor is on by default; if it is down, the node falls back to clearnet and says so.

No premine. No company. No admin key. No official price — mine it or receive it.

**How to be here**
1. Read <#RULES>
2. Get the wallet in <#START>
3. Talk in <#GENERAL> (English) or <#DEUTSCH>

Website: https://nightfallcoin.org
Source: https://github.com/Instinctes/nightfall
Network: https://nightfallcoin.org/emission/

Staff will **never** DM you first. Anyone who does, or who asks for your 24 words, is a thief.""",
    "rules": """**Rules**

1. Never paste a seed, a spending key, or an `nfview1` key. Not here, not in DMs.
2. Staff never writes you first and will never ask you to “verify” a wallet.
3. No official price, no tickers, no “wen listing.” A number we invented would be a lie.
4. Do not call this untraceable or 100% anonymous. It is not.
5. English in <#GENERAL>. German in <#DEUTSCH>. Say OS, wallet version, what you expected.
6. No scams, impersonation, airdrops, paid groups, or fake admins.
7. Do not mine with zero peers. Two lonely miners build two chains; the lighter one dies.
8. Ban is for theft and impersonation. Mute is for noise.
9. Unofficial sites, pools, bots, and “inbox me” are not us unless they were posted in <#ANNOUNCEMENTS>.

Desktop **0.8.1**. Phone and the browser wallet **0.7.0**. Same chain.""",
    "announcements": """NIGHTFALL Discord is open.

Same chain as the website. Desktop **0.8.1**, phone/web **0.7.0**, seed **0.8.0**.

If something is on fire — a release, a seed incident, a chain notice — it will be posted **here**. A message in #general is not an official statement. A pool, ticker, or DM is not us.""",
    "releases": """**Current builds**

• Desktop Core **0.8.1** — macOS / Windows / Linux
  https://github.com/Instinctes/nightfall/releases/tag/v0.8.1
• Phone APK / iOS IPA / web wallet **0.7.0**
• Seed node **0.8.0** (same chain; does not mine)

Downloads + SHA-256: https://nightfallcoin.org/#download

Same chain: genesis `061a052d…` · protocol v8 · wire v6 · magic `NFL2`.
Wallets 0.6.x are a buried chain (`NFL1`). They cannot handshake. Those coins are not NIGHT here.

0.8.1 is a wallet release — no consensus change. 0.7.x and 0.8.0 peer with it normally.

Builds are unsigned. macOS: right-click → Open. Windows: More info → Run anyway.""",
    "start-here": """**Get NIGHT**

1. Download Core from https://nightfallcoin.org/#download
2. Check the file against `SHA256SUMS-0.8.1.txt` (Windows/Linux have their own lists).
3. Open it. Connect. **Then** mine. The seed is `seed.nightfallcoin.org:17891`.

The Core wallet is a full node. No server to trust, nothing to configure.

Phone and the browser wallet at https://nightfallcoin.org/wallet/ stay on **0.7.0**. They trust a node for what they *show*. They cannot spend — the 24 words never leave the device. Write them down. The iOS IPA is unsigned; AltStore / Sideloadly / Xcode, or just use the web wallet.

**Windows sits on BLOCKS = 1**
Allow inbound TCP **17891** or Defender will keep you alone on genesis.

**“1 behind, cannot mine”**
Often a short fork, not a dead connection. Core 0.8.1 is honest about that (catching up / competing tip / dead branch) and can resync the chain while keeping the wallet.

**Stuck on “pending” after a send**
The coins were never spent. Settings → **Rescan from genesis** releases them. Don't send again before the rescan.

Do not delete the chain file to “fix” a fork (`blocks.bin` after 0.8.0, or `blocks.jsonl` if you have not converted). A coinbase on the branch you abandoned does not come back.""",
    "links": """**Official**

• Website — https://nightfallcoin.org
• Source — https://github.com/Instinctes/nightfall
• Releases — https://github.com/Instinctes/nightfall/releases
• Current desktop — https://github.com/Instinctes/nightfall/releases/tag/v0.8.1
• Emission / live numbers (no addresses) — https://nightfallcoin.org/emission/
• Spec — https://github.com/Instinctes/nightfall/blob/main/docs/SPEC.md
• Honest limitations — https://nightfallcoin.org/#honest
• Internal review (not an outside audit) — https://nightfallcoin.org/audit/
• Bitcointalk ANN — https://bitcointalk.org/index.php?topic=5591473.0

If a site, bot, or DM is not in this list, it is not us.""",
    "deutsch": """Deutscher Raum. Dieselbe Kette, dieselben Regeln.

Kein Seed, kein Spend-Key, kein `nfview1` hier oder in DMs.
Kein Preis, kein Ticker, kein „100 % anonym“.
Staff schreibt dich nicht zuerst an.

Desktop **0.8.1**, Handy/Web **0.7.0**. Website: https://nightfallcoin.org

Windows auf Block 1: Port **17891** in der Firewall freigeben.""",
    "mining": """**Mining**

Nighthash-v2 is Argon2id: 32 MiB of RAM per hash. Target ~15 s. No pool. Core Wallet → Start mining. It leaves one CPU core free.

Connect to a peer first. Two miners who never meet build two chains from the same genesis; the lighter one is discarded.

Rewards unlock after **1,440 blocks** (~six hours). That delay is for reorgs, not a lock-up.

The dashboard “network hash” is `difficulty ÷ 15`. An estimate. Not a miner headcount. Coinbase does not reveal who hashed the block.

Founder mines little on purpose. A small chain reorgs easily — that is listed as a risk, not a secret.

A pool, miner, or website that was not posted in <#ANNOUNCEMENTS> is someone else's. Ask in <#HELP> before you point a machine at it.""",
    "nodes": """**Running a node**

P2P `17891`
RPC `127.0.0.1:17881` — newline JSON, **not HTTP**. Method is `status`, not `getinfo`.

```
python3 -c 'import socket; s=socket.create_connection(("127.0.0.1",17881),5); s.sendall(b"{\\"method\\":\\"status\\",\\"params\\":{},\\"id\\":1}\\n"); print(s.recv(8192).decode())'
```

Public height / difficulty / supply: https://nightfallcoin.org/emission/
Linux install notes: https://nightfallcoin.org/build/#linux

Datadir for current protocol: `nightfall/<net>/n8/`. Old v7 data stays next to it and is not this chain.""",
    "help": """**Help**

Include: OS, which app (Core / phone / web), version, what you see, what you expected.

Known, already seen:
• Windows BLOCKS = 1, PEERS = 0 → port **17891** / Defender.
• “1 behind, cannot mine” → fork hold (`stalled_on_fork`), not the socket. Update to **0.8.1**.
• Send stuck on “pending” → coins were never spent. Settings → **Rescan from genesis**. Don't send again first.
• Tor off → clearnet fallback, the wallet says `tor_proxy: false`. That is honest, not a bug.
• 0.6.x cannot join this chain.

Do not paste 24 words, hex seeds, or view keys. If you already did, the wallet is burned — make a new one, move what you can.""",
    "bugs": """**Bugs**

One report per message. Version + OS + steps + expected vs actual.

The v4 inflation class is closed. Replaying it is `cargo test -p nightfall-ledger --test exploit_regression`. Passing tests are not an outside audit.

No seeds in this channel.""",
    "dev": """**Development**

Protocol **v8** · wire **v6** · magic **NFL2**
Genesis `061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de`

Repo: https://github.com/Instinctes/nightfall
Spec: `docs/SPEC.md`
Do not soften `exploit_regression.rs`.

Desktop is 0.8.1. Phone/web stay 0.7.0 until the light API changes. Fees stay burned while subsidy > 0. Mempool policy is not a playground.""",
}


class Discord:
    def __init__(self, token: str) -> None:
        self.token = token.strip()
        self._me: dict[str, Any] | None = None

    def req(
        self,
        method: str,
        path: str,
        body: Any | None = None,
        *,
        files: bool = False,
    ) -> Any:
        data = None
        headers = {
            "Authorization": f"Bot {self.token}",
            "User-Agent": "NightfallDiscordSetup (https://nightfallcoin.org, 1)",
        }
        if body is not None:
            data = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        url = API + path
        for attempt in range(8):
            req = urllib.request.Request(url, data=data, headers=headers, method=method)
            try:
                with urllib.request.urlopen(req, timeout=30, context=SSL_CTX) as res:
                    raw = res.read()
                    if not raw:
                        return None
                    return json.loads(raw)
            except urllib.error.HTTPError as e:
                raw = e.read()
                if e.code == 429:
                    retry = 1.0
                    try:
                        retry = float(json.loads(raw).get("retry_after", 1))
                    except Exception:
                        pass
                    time.sleep(retry + 0.05)
                    continue
                try:
                    err = json.loads(raw)
                except Exception:
                    err = raw.decode("utf-8", "replace")
                raise SystemExit(f"{method} {path} → {e.code}: {err}") from None
            except urllib.error.URLError as e:
                if attempt == 7:
                    raise SystemExit(f"{method} {path} → {e}") from None
                time.sleep(1.5 * (attempt + 1))
        raise SystemExit(f"{method} {path} → gave up after rate limits")

    def me(self) -> dict[str, Any]:
        if self._me is None:
            self._me = self.req("GET", "/users/@me")
        return self._me


def overwrite(role_id: str, *, allow: int = 0, deny: int = 0) -> dict[str, Any]:
    return {"id": role_id, "type": 0, "allow": str(allow), "deny": str(deny)}


def load_icon() -> str | None:
    path = os.path.abspath(ICON_PATH)
    if not os.path.isfile(path):
        print(f"no icon at {path}")
        return None
    with open(path, "rb") as f:
        b64 = base64.b64encode(f.read()).decode()
    return f"data:image/png;base64,{b64}"


def channel_type(kind: str, news_ok: bool) -> int:
    if kind == "voice":
        return 2
    if kind == "news" and news_ok:
        return 5
    return 0


def find_channel(
    channels: list[dict[str, Any]],
    name: str,
    aliases: list[str] | None = None,
    ctype: int | None = None,
) -> dict[str, Any] | None:
    want = {name.lower()}
    if aliases:
        want.update(a.lower() for a in aliases)
    matches: list[dict[str, Any]] = []
    for c in channels:
        if c.get("type") == 4:
            continue
        if ctype is not None and c.get("type") != ctype:
            continue
        if c.get("name", "").lower() in want:
            matches.append(c)
    exact = [c for c in matches if c.get("name", "").lower() == name.lower()]
    return (exact or matches or [None])[0]


def find_category(channels: list[dict[str, Any]], name: str) -> dict[str, Any] | None:
    for c in channels:
        if c.get("type") == 4 and c.get("name") == name:
            return c
    return None


def find_role(roles: list[dict[str, Any]], name: str) -> dict[str, Any] | None:
    for r in roles:
        if r.get("name") == name:
            return r
    return None


def fill_mentions(text: str, ids: dict[str, str]) -> str:
    mapping = {
        "<#RULES>": ids.get("rules"),
        "<#START>": ids.get("start-here"),
        "<#GENERAL>": ids.get("general"),
        "<#DEUTSCH>": ids.get("deutsch"),
        "<#ANNOUNCEMENTS>": ids.get("announcements"),
        "<#HELP>": ids.get("help"),
    }
    out = text
    for token, cid in mapping.items():
        if cid:
            out = out.replace(token, f"<#{cid}>")
        else:
            out = out.replace(token, "#" + token[2:-1].lower().replace("start", "start-here"))
    return out


def pin_if_needed(api: Discord, channel_id: str, content: str) -> None:
    """Keep exactly one pinned copy of this body. Replace a stale pin whose
    first line matches (so a version bump in #releases does not stack)."""
    pins = api.req("GET", f"/channels/{channel_id}/pins") or []
    head = content.strip().splitlines()[0][:48]
    for p in pins:
        body = p.get("content") or ""
        if body == content:
            print(f"  pin current in {channel_id}")
            return
    for p in pins:
        body = p.get("content") or ""
        if head and body.startswith(head[:24]):
            try:
                api.req("DELETE", f"/channels/{channel_id}/pins/{p['id']}")
            except SystemExit:
                pass
    recent = api.req("GET", f"/channels/{channel_id}/messages?limit=20") or []
    for m in recent:
        if m.get("content") == content:
            api.req("PUT", f"/channels/{channel_id}/pins/{m['id']}")
            print(f"  pinned existing message in {channel_id}")
            return
    msg = api.req("POST", f"/channels/{channel_id}/messages", {"content": content})
    api.req("PUT", f"/channels/{channel_id}/pins/{msg['id']}")
    print(f"  pinned new message in {channel_id}")
    time.sleep(0.3)


def main() -> int:
    token = os.environ.get("DISCORD_BOT_TOKEN") or (sys.argv[1] if len(sys.argv) > 1 else "")
    if not token:
        print("usage: DISCORD_BOT_TOKEN=... python3 scripts/setup-discord.py", file=sys.stderr)
        return 2

    api = Discord(token)
    me = api.me()
    print(f"bot @{me.get('username')} ({me.get('id')})")

    guild = api.req("GET", f"/guilds/{GUILD_ID}")
    print(f"guild {guild.get('name')} owner={guild.get('owner_id')}")

    icon = load_icon()
    patch_guild: dict[str, Any] = {
        "name": "NIGHTFALL",
        "description": (
            "Money that refuses to snitch. Privacy Layer-1, zero premine, "
            "CPU-mineable. Not 100% anonymous. No official price."
        ),
        "verification_level": 2,  # 5 minutes on Discord — stops most raid accounts
        "explicit_content_filter": 2,  # scan all members
        "default_message_notifications": 1,  # only mentions
        "preferred_locale": "en-US",
        "afk_timeout": 300,
    }
    if icon:
        patch_guild["icon"] = icon
    try:
        guild = api.req("PATCH", f"/guilds/{GUILD_ID}", patch_guild)
        print("updated guild profile")
    except SystemExit as e:
        # Icon or description can fail on tiny / unverified bots; retry without icon.
        print(f"guild patch failed ({e}); retrying without icon/description")
        patch_guild.pop("icon", None)
        patch_guild.pop("description", None)
        guild = api.req("PATCH", f"/guilds/{GUILD_ID}", patch_guild)

    roles = api.req("GET", f"/guilds/{GUILD_ID}/roles")
    role_ids: dict[str, str] = {}
    for spec in ROLES:
        existing = find_role(roles, spec["name"])
        body = {
            "name": spec["name"],
            "color": spec["color"],
            "hoist": spec["hoist"],
            "mentionable": spec["mentionable"],
            "permissions": spec["permissions"],
        }
        if existing:
            r = api.req("PATCH", f"/guilds/{GUILD_ID}/roles/{existing['id']}", body)
            print(f"role updated: {spec['name']}")
        else:
            r = api.req("POST", f"/guilds/{GUILD_ID}/roles", body)
            print(f"role created: {spec['name']}")
            roles.append(r)
        role_ids[spec["name"]] = r["id"]
        time.sleep(0.2)

    everyone = GUILD_ID
    # @everyone: no @everyone pings, otherwise a normal community.
    api.req(
        "PATCH",
        f"/guilds/{GUILD_ID}/roles/{everyone}",
        {
            "permissions": str(EVERYONE_ALLOW),
        },
    )
    print("updated @everyone permissions")

    founder_id = role_ids["Founder"]
    owner_id = guild.get("owner_id")
    if owner_id:
        try:
            api.req(
                "PUT",
                f"/guilds/{GUILD_ID}/members/{owner_id}/roles/{founder_id}",
                {},
            )
            print("assigned Founder to guild owner")
        except SystemExit as e:
            print(f"could not assign Founder (bot role must sit above it): {e}")

    channels = api.req("GET", f"/guilds/{GUILD_ID}/channels")
    news_ok = False
    cat_ids: dict[str, str] = {}
    chan_ids: dict[str, str] = {}
    staff_role_ids = [role_ids["Founder"], role_ids["Moderator"]]

    def staff_overwrites(parent_everyone_deny_view: bool, readonly: bool) -> list[dict[str, Any]]:
        ows = []
        if parent_everyone_deny_view:
            ows.append(overwrite(everyone, deny=P_VIEW_CHANNEL))
            for rid in staff_role_ids:
                ows.append(overwrite(rid, allow=P_VIEW_CHANNEL | STAFF_ALLOW))
        elif readonly:
            ows.append(overwrite(everyone, deny=INFO_DENY_SEND))
            for rid in staff_role_ids:
                ows.append(overwrite(rid, allow=P_SEND_MESSAGES | P_MANAGE_MESSAGES | P_EMBED_LINKS))
        return ows

    position = 0
    for cat in LAYOUT:
        staff_only = bool(cat.get("staff_only"))
        existing_cat = find_category(channels, cat["name"])
        cat_body: dict[str, Any] = {
            "name": cat["name"],
            "type": 4,
            "position": position,
        }
        cow = staff_overwrites(staff_only, False)
        if cow:
            cat_body["permission_overwrites"] = cow
        if existing_cat:
            c = api.req("PATCH", f"/channels/{existing_cat['id']}", cat_body)
            print(f"category updated: {cat['name']}")
        else:
            cat_body["guild_id"] = GUILD_ID
            # Creating a category uses POST /guilds/{id}/channels
            c = api.req(
                "POST",
                f"/guilds/{GUILD_ID}/channels",
                {
                    "name": cat["name"],
                    "type": 4,
                    "position": position,
                    **({"permission_overwrites": cow} if cow else {}),
                },
            )
            print(f"category created: {cat['name']}")
            channels.append(c)
        cat_ids[cat["name"]] = c["id"]
        parent = c["id"]
        position += 1
        time.sleep(0.15)

        for spec in cat["channels"]:
            kind = spec["kind"]
            ctype = channel_type(kind, news_ok)
            aliases = spec.get("alias") or []
            # Always recognise the known default #Allgemein as general.
            if spec["name"] == "general":
                aliases = list(set(aliases + ["allgemein"]))
            existing = find_channel(channels, spec["name"], aliases, ctype=ctype)
            ows = staff_overwrites(staff_only, bool(spec.get("readonly")))
            body: dict[str, Any] = {
                "name": spec["name"],
                "type": ctype,
                "parent_id": parent,
                "position": position,
            }
            if kind != "voice":
                body["topic"] = spec.get("topic") or ""
            if ows:
                body["permission_overwrites"] = ows
            if existing:
                # Cannot always change type 0 → 5 without community; leave type.
                patch = {
                    "name": spec["name"],
                    "parent_id": parent,
                    "position": position,
                }
                if kind != "voice":
                    patch["topic"] = spec.get("topic") or ""
                if ows:
                    patch["permission_overwrites"] = ows
                ch = api.req("PATCH", f"/channels/{existing['id']}", patch)
                print(f"  channel updated: #{spec['name']}")
            else:
                create = {
                    "name": spec["name"],
                    "type": ctype,
                    "parent_id": parent,
                    "position": position,
                }
                if kind != "voice":
                    create["topic"] = spec.get("topic") or ""
                if ows:
                    create["permission_overwrites"] = ows
                ch = api.req("POST", f"/guilds/{GUILD_ID}/channels", create)
                print(f"  channel created: #{spec['name']}")
                channels.append(ch)
            chan_ids[spec["name"]] = ch["id"]
            position += 1
            time.sleep(0.15)

    # Try to turn on Community so #announcements can be a News channel
    # and the welcome screen works. Safe to skip if Discord refuses.
    rules_id = chan_ids.get("rules")
    modlog_id = chan_ids.get("mod-log")
    welcome_id = chan_ids.get("welcome")
    general_id = chan_ids.get("general")
    community_on = False
    if rules_id and modlog_id:
        try:
            api.req(
                "PATCH",
                f"/guilds/{GUILD_ID}",
                {
                    "features": ["COMMUNITY"],
                    "rules_channel_id": rules_id,
                    "public_updates_channel_id": modlog_id,
                    "explicit_content_filter": 2,
                    "verification_level": 2,
                    "preferred_locale": "en-US",
                },
            )
            community_on = True
            print("community mode on")
        except SystemExit as e:
            print(f"community mode skipped: {e}")

    if community_on and "announcements" in chan_ids:
        try:
            api.req(
                "PATCH",
                f"/channels/{chan_ids['announcements']}",
                {"type": 5, "topic": LAYOUT[0]["channels"][2]["topic"]},
            )
            print("announcements is a news channel")
        except SystemExit as e:
            print(f"could not convert announcements to news: {e}")

    # Join-spam in #welcome is how a crypto server starts looking like a
    # casino lobby. Suppress every system message type Discord offers, and
    # park whatever remains in #mod-log — never in a read-only info channel.
    flags = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3)
    try:
        api.req(
            "PATCH",
            f"/guilds/{GUILD_ID}",
            {
                "system_channel_id": modlog_id or welcome_id,
                "system_channel_flags": flags,
                "afk_channel_id": chan_ids.get("Lounge"),
            },
        )
        print("system messages → #mod-log, joins suppressed")
    except SystemExit as e:
        print(f"system channel skipped: {e}")

    if community_on and welcome_id:
        welcome_channels = []
        for name, desc, emoji in (
            ("rules", "Read this first.", "📜"),
            ("start-here", "Wallet, checksums, how to mine.", "⬇️"),
            ("announcements", "Official only.", "📣"),
            ("general", "English chat.", "💬"),
        ):
            cid = chan_ids.get(name)
            if cid:
                welcome_channels.append(
                    {
                        "channel_id": cid,
                        "description": desc,
                        "emoji_name": emoji,
                    }
                )
        try:
            api.req(
                "PATCH",
                f"/guilds/{GUILD_ID}/welcome-screen",
                {
                    "enabled": True,
                    "description": "Privacy Layer-1. Zero premine. Not 100% anonymous.",
                    "welcome_channels": welcome_channels[:5],
                },
            )
            print("welcome screen set")
        except SystemExit as e:
            print(f"welcome screen skipped: {e}")

    # AutoMod: block the messages that drain wallets, alert #mod-log.
    # Keyword lists are short on purpose — a false block of a support
    # thread is worse than missing a paraphrased scam once.
    if modlog_id:
        automod_rules = [
            {
                "name": "wallet-drain",
                "event_type": 1,
                "trigger_type": 1,
                "trigger_metadata": {
                    "keyword_filter": [
                        "nfview1",
                        "miner.seed",
                        "seed phrase",
                        "recovery phrase",
                        "24 words",
                        "private key",
                        "send your seed",
                        "paste your seed",
                        "verify your wallet",
                        "verify your seed",
                        "airdrop dm",
                        "free night",
                    ],
                    "allow_list": [],
                },
                "actions": [
                    {"type": 1, "metadata": {"custom_message": "Do not paste keys. Staff never DMs first."}},
                    {"type": 2, "metadata": {"channel_id": modlog_id}},
                ],
                "enabled": True,
            },
            {
                "name": "mention-spam",
                "event_type": 1,
                "trigger_type": 5,
                "trigger_metadata": {"mention_total_limit": 5},
                "actions": [
                    {"type": 1, "metadata": {"custom_message": "Too many mentions."}},
                    {"type": 2, "metadata": {"channel_id": modlog_id}},
                ],
                "enabled": True,
            },
        ]
        existing_am = []
        try:
            existing_am = api.req("GET", f"/guilds/{GUILD_ID}/auto-moderation/rules") or []
        except SystemExit as e:
            print(f"automod list skipped: {e}")
            existing_am = []
        by_name = {r.get("name"): r for r in existing_am}
        for spec in automod_rules:
            have = by_name.get(spec["name"])
            try:
                if have:
                    api.req(
                        "PATCH",
                        f"/guilds/{GUILD_ID}/auto-moderation/rules/{have['id']}",
                        spec,
                    )
                    print(f"automod updated: {spec['name']}")
                else:
                    api.req(
                        "POST",
                        f"/guilds/{GUILD_ID}/auto-moderation/rules",
                        spec,
                    )
                    print(f"automod created: {spec['name']}")
            except SystemExit as e:
                print(f"automod {spec['name']} skipped: {e}")
            time.sleep(0.2)

    # Pins — after we know every channel id so mentions resolve.
    for cat in LAYOUT:
        for spec in cat["channels"]:
            key = spec.get("pin")
            if not key:
                continue
            cid = chan_ids.get(spec["name"])
            body = PINS.get(key)
            if not cid or not body:
                continue
            pin_if_needed(api, cid, fill_mentions(body, chan_ids))

    # Permanent invite on #welcome (the public website one currently expires).
    invite_channel = chan_ids.get("welcome") or chan_ids.get("general")
    invite_url = None
    if invite_channel:
        inv = api.req(
            "POST",
            f"/channels/{invite_channel}/invites",
            {
                "max_age": 0,
                "max_uses": 0,
                "temporary": False,
                "unique": False,
            },
        )
        code = inv.get("code")
        invite_url = f"https://discord.gg/{code}"
        print(f"permanent invite: {invite_url}")

    # Drop Discord's empty default categories after the layout exists.
    channels = api.req("GET", f"/guilds/{GUILD_ID}/channels")
    for leftover in ("Textkanäle", "Sprachkanäle"):
        cat = find_category(channels, leftover)
        if not cat:
            continue
        kids = [c for c in channels if c.get("parent_id") == cat["id"]]
        if kids:
            print(f"left {leftover} (still has {len(kids)} channel(s))")
            continue
        try:
            api.req("DELETE", f"/channels/{cat['id']}")
            print(f"deleted leftover category {leftover}")
        except SystemExit as e:
            print(f"could not delete {leftover}: {e}")

    # Bot avatar = site logo.
    icon = load_icon()
    if icon:
        try:
            api.req("PATCH", "/users/@me", {"avatar": icon})
            print("updated bot avatar")
        except SystemExit as e:
            print(f"bot avatar skipped: {e}")

    print("\n--- done ---")
    print("channels:")
    for n, i in chan_ids.items():
        print(f"  #{n}  {i}")
    if invite_url:
        print(f"invite {invite_url}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
