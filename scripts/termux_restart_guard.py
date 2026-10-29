#!/usr/bin/env python3

"""Read-only Nightfall v8 restart preflight."""

import argparse
import json
import os
import stat
import sys

from pathlib import Path

GENESIS = (
    "061a052d49607ff8f4b306c75d622eb"
    "d230cff4ec3a45a6dffc2f7738d4b20de"
)


def inspect(root, mining=False, miner_seed="miner.seed"):

    root = Path(root).expanduser()
    checks = []

    def add(name, ok, detail, review=False):
        checks.append({
            "check": name,
            "status": (
                "PASS" if ok
                else "REVIEW" if review
                else "BLOCK"
            ),
            "detail": detail
        })

    if root.is_symlink() or not root.is_dir():

        add(
            "DATADIR",
            False,
            "Existing ordinary directory required"
        )

        return result(checks)

    add(
        "DATADIR",
        True,
        "Existing directory"
    )

    recovery = root / ".recovery-journal.json"

    add(
        "RECOVERY",
        not (
            recovery.exists()
            or recovery.is_symlink()
        ),
        "Interrupted recovery marker must not exist"
    )

    legacy = root / "chain.json"

    add(
        "LEGACY",
        not (
            legacy.exists()
            or legacy.is_symlink()
        ),
        "Old protocol chain marker must not exist"
    )

    binary = root / "blocks.bin"
    text = root / "blocks.jsonl"

    has_bin = (
        binary.exists()
        or binary.is_symlink()
    )

    has_text = (
        text.exists()
        or text.is_symlink()
    )

    chosen = binary if has_bin else text

    valid_file = (
        (has_bin or has_text)
        and not chosen.is_symlink()
        and chosen.is_file()
    )

    length = None

    if valid_file:
        try:
            length = chosen.stat().st_size
        except OSError:
            pass

    add(
        "CHAIN",
        length is not None and length > 0,
        "Saved nonempty chain file required"
    )

    if has_bin and has_text:

        add(
            "FORMATS",
            False,
            "Both formats exist; binary normally wins",
            review=True
        )

    meta_file = root / "chain-meta.json"
    meta = None

    try:

        if (
            meta_file.is_symlink()
            or not meta_file.is_file()
        ):
            raise ValueError("missing or symlink")

        with meta_file.open("rb") as f:
            raw = f.read(65537)

        if len(raw) > 65536:
            raise ValueError("too large")

        meta = json.loads(raw)

        if not isinstance(meta, dict):
            raise ValueError("not an object")

    except (OSError, ValueError, UnicodeError):
        pass

    add(
        "METADATA",
        meta is not None,
        "Valid bounded chain metadata required"
    )

    if meta is not None:

        add(
            "NETWORK",
            meta.get("network") == "mainnet",
            "Expected mainnet"
        )

        pv = meta.get("protocol_version")

        add(
            "PROTOCOL",
            type(pv) is int and pv == 8,
            "Expected protocol v8"
        )

        add(
            "GENESIS",
            meta.get("genesis_hash") == GENESIS,
            "Expected v8 genesis"
        )

        bc = meta.get("block_count")

        add(
            "PROGRESS",
            type(bc) is int and bc > 1,
            "Saved chain must extend beyond genesis"
        )

        vt = meta.get("validated_tip")

        good_tip = (
            isinstance(vt, str)
            and len(vt) == 64
            and all(
                c in "0123456789abcdef"
                for c in vt
            )
        )

        add(
            "TIP",
            good_tip,
            "Expected 64-character tip",
            review=True
        )

        vbytes = meta.get("validated_bytes")

        size_ok = (
            type(vbytes) is int
            and length is not None
            and vbytes == length
            and length > 0
        )

        add(
            "LENGTH",
            size_ok,
            "Reported and actual chain sizes agree",
            review=True
        )

        try:

            id_file = root / "install-id"

            if (
                id_file.is_symlink()
                or not id_file.is_file()
            ):
                raise ValueError("missing or symlink")

            with id_file.open("rb") as f:
                local_id = (
                    f.read(257)
                    .decode("ascii")
                    .strip()
                )

            if len(local_id) > 256:
                local_id = ""

        except (OSError, ValueError, UnicodeError):
            local_id = ""

        add(
            "INSTALL_ID",
            bool(local_id)
            and meta.get("validated_by") == local_id,
            "Local validation marker agrees",
            review=True
        )

    if mining:

        seed = Path(miner_seed).expanduser()

        if not seed.is_absolute():
            seed = root / seed

        try:

            mode = seed.lstat()

            present = (
                stat.S_ISREG(mode.st_mode)
                and mode.st_size > 0
            )

            private = (
                present
                and not (mode.st_mode & 0o077)
            )

        except OSError:
            present = False
            private = False

        add(
            "MINER_SEED",
            present,
            "Existing nonempty miner seed required"
        )

        if present:

            add(
                "SEED_PERMISSIONS",
                private,
                "Seed permissions should be private",
                review=True
            )

    try:

        st = os.statvfs(root)

        free = (
            st.f_bavail
            * st.f_frsize
        )

        add(
            "DISK_SPACE",
            free >= 256 * 1024 * 1024,
            "256 MiB free recommended",
            review=True
        )

    except OSError:

        add(
            "DISK_SPACE",
            False,
            "Free space unknown",
            review=True
        )

    return result(checks)


def result(checks):

    statuses = {
        row["status"]
        for row in checks
    }

    if "BLOCK" in statuses:
        overall = "BLOCKED"

    elif "REVIEW" in statuses:
        overall = "REVIEW"

    else:
        overall = "READY"

    return {
        "schema": "nightfall-termux-restart-guard-v1",
        "result": overall,
        "checks": checks,
        "scope": "local-metadata-only"
    }


def main():

    p = argparse.ArgumentParser(
        description=__doc__
    )

    p.add_argument(
        "--datadir",
        required=True
    )

    p.add_argument(
        "--mining",
        action="store_true"
    )

    p.add_argument(
        "--miner-seed",
        default="miner.seed"
    )

    p.add_argument(
        "--json",
        action="store_true"
    )

    a = p.parse_args()

    data = inspect(
        a.datadir,
        a.mining,
        a.miner_seed
    )

    if a.json:

        print(
            json.dumps(
                data,
                sort_keys=True
            )
        )

    else:

        print(
            "Nightfall restart preflight:",
            data["result"]
        )

        for c in data["checks"]:

            print(
                "{:<6} {:<18} {}".format(
                    c["status"],
                    c["check"],
                    c["detail"]
                )
            )

        print(
            "Read-only metadata check; "
            "no reset or PoW verification."
        )

    return {
        "READY": 0,
        "REVIEW": 1,
        "BLOCKED": 2
    }[data["result"]]


if __name__ == "__main__":
    sys.exit(main())
