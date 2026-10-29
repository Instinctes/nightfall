#!/usr/bin/env python3

"""
Offline, non-consensus Nightpool PPLNS
work-accounting reference.

WARNING:
Work and share IDs are unverified inputs.

This software does not validate Nightfall PoW,
create transactions, pay miners or handle funds.
"""

import argparse
import hashlib
import json
import re
import sys

from pathlib import Path


SCHEMA = "nightpool-work-accounting-research-v0"

MAX_INPUT_BYTES = 4_000_000
MAX_SHARES = 50_000

MAX_U128 = (1 << 128) - 1
MAX_U64 = (1 << 64) - 1

REWARD_DARKS = 600_000_000

ZERO_ID = "0" * 64

HEX64 = re.compile(
    r"[0-9a-f]{64}\Z"
)

MINER = re.compile(
    r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,63}\Z"
)


class InputError(ValueError):
    pass


def strict_pairs(pairs):

    value = {}

    for key, item in pairs:

        if key in value:
            raise InputError(
                "duplicate JSON key"
            )

        value[key] = item

    return value


def parse_json(data):

    try:

        return json.loads(
            data,
            object_pairs_hook=strict_pairs,
            parse_constant=lambda _: (
                _ for _ in ()
            ).throw(
                InputError(
                    "invalid numeric constant"
                )
            ),
        )

    except (
        UnicodeError,
        ValueError,
        RecursionError
    ) as exc:

        raise InputError(
            "malformed JSON"
        ) from exc


def pos_int(value, label, limit):

    if (
        type(value) is not int
        or not 1 <= value <= limit
    ):
        raise InputError(
            f"{label} must be an integer in 1..{limit}"
        )

    return value


def validate(document):

    required = {
        "schema",
        "window_work",
        "shares"
    }

    if (
        not isinstance(document, dict)
        or set(document) != required
    ):
        raise InputError(
            "document fields do not match research schema"
        )

    if document["schema"] != SCHEMA:
        raise InputError(
            "unexpected research schema"
        )

    window = pos_int(
        document["window_work"],
        "window_work",
        MAX_U128
    )

    shares = document["shares"]

    if (
        not isinstance(shares, list)
        or not 1 <= len(shares) <= MAX_SHARES
    ):
        raise InputError(
            "shares must be a nonempty bounded array"
        )

    previous = ZERO_ID
    seen = set()
    total = 0

    for share in shares:

        fields = {
            "id",
            "prev",
            "miner",
            "work"
        }

        if (
            not isinstance(share, dict)
            or set(share) != fields
        ):
            raise InputError(
                "share has unexpected or missing fields"
            )

        ident = share["id"]
        parent = share["prev"]
        miner = share["miner"]

        if (
            type(ident) is not str
            or not HEX64.fullmatch(ident)
            or ident == ZERO_ID
        ):
            raise InputError(
                "invalid share identifier"
            )

        if ident in seen:
            raise InputError(
                "replayed share identifier"
            )

        if (
            type(parent) is not str
            or parent != previous
        ):
            raise InputError(
                "missing, divergent or reordered share parent"
            )

        if (
            type(miner) is not str
            or not MINER.fullmatch(miner)
        ):
            raise InputError(
                "invalid synthetic miner label"
            )

        work = pos_int(
            share["work"],
            "work",
            MAX_U64
        )

        total += work

        if total > MAX_U128:
            raise InputError(
                "total claimed work exceeds u128"
            )

        seen.add(ident)

        previous = ident

    return window, shares, total


def allocate(weights, reward):

    """
    Largest-remainder integer allocation.
    Deterministic on equal remainders.
    """

    pos_int(
        reward,
        "reward",
        REWARD_DARKS
    )

    denominator = sum(
        weights.values()
    )

    if denominator <= 0:
        raise InputError(
            "zero credited work"
        )

    payouts = {}
    residuals = []

    floor_total = 0

    for miner in sorted(weights):

        numerator = (
            reward * weights[miner]
        )

        floor, remainder = divmod(
            numerator,
            denominator
        )

        payouts[miner] = floor

        floor_total += floor

        residuals.append(
            (-remainder, miner)
        )

    outstanding = (
        reward - floor_total
    )

    for _, miner in sorted(
        residuals
    )[:outstanding]:

        payouts[miner] += 1

    assert sum(
        payouts.values()
    ) == reward

    return payouts


def calculate(
    document,
    reward=REWARD_DARKS
):

    """
    Replay a single claimed share chain.

    This is NOT proof-of-work validation.
    """

    window, shares, total = validate(
        document
    )

    pos_int(
        reward,
        "reward",
        REWARD_DARKS
    )

    remaining = min(
        window,
        total
    )

    weights = {}

    for share in reversed(shares):

        credited = min(
            share["work"],
            remaining
        )

        if credited:

            miner = share["miner"]

            weights[miner] = (
                weights.get(miner, 0)
                + credited
            )

            remaining -= credited

        if remaining == 0:
            break

    used = sum(
        weights.values()
    )

    payouts = allocate(
        weights,
        reward
    )

    result = {

        "schema": SCHEMA,

        "status":
            "RESEARCH_ONLY_UNVERIFIED_SHARES",

        "reward_darks":
            reward,

        "window_work":
            window,

        "credited_work":
            used,

        "claimed_chain_work":
            total,

        "payouts": [

            {
                "miner": name,

                "credited_work":
                    weights[name],

                "darks":
                    payouts[name]
            }

            for name in sorted(weights)

        ],

        "conservation_ok":
            sum(payouts.values()) == reward
    }

    return result


def digest(value):

    payload = json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True
    )

    return hashlib.sha256(
        payload.encode("ascii")
    ).hexdigest()


def demo():

    values = [
        ("alice", 5),
        ("bob", 4),
        ("alice", 3)
    ]

    shares = []

    for n, (name, work) in enumerate(
        values,
        1
    ):

        shares.append({

            "id":
                f"{n:064x}",

            "prev":
                f"{n-1:064x}",

            "miner":
                name,

            "work":
                work
        })

    return {

        "schema":
            SCHEMA,

        "window_work":
            10,

        "shares":
            shares
    }


def main(argv=None):

    parser = argparse.ArgumentParser(
        description=(
            "Nightpool offline payout research; "
            "NOT a mining pool"
        )
    )

    parser.add_argument(
        "--demo",
        action="store_true"
    )

    parser.add_argument(
        "--input",
        type=Path
    )

    args = parser.parse_args(argv)

    if args.demo == bool(args.input):

        parser.error(
            "provide exactly one of --demo or --input"
        )

    try:

        if args.demo:

            source = demo()

        else:

            if args.input.is_symlink():
                raise InputError(
                    "refusing symlink input"
                )

            with args.input.open("rb") as f:

                raw = f.read(
                    MAX_INPUT_BYTES + 1
                )

            if len(raw) > MAX_INPUT_BYTES:
                raise InputError(
                    "input larger than 4 MB"
                )

            source = parse_json(
                raw.decode(
                    "utf-8",
                    "strict"
                )
            )

        report = calculate(
            source
        )

        report["input_sha256"] = digest(
            source
        )

        print(
            json.dumps(
                report,
                indent=2,
                sort_keys=True
            )
        )

        return 0

    except (
        InputError,
        OSError,
        UnicodeError
    ) as exc:

        print(
            f"nightpool research: {exc}",
            file=sys.stderr
        )

        return 2


if __name__ == "__main__":
    sys.exit(main())
