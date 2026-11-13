#!/usr/bin/env python3
"""Offline adversarial invariants for Nightwatch."""

import json
import tempfile
import unittest

from pathlib import Path
from unittest.mock import patch

import nightwatch as nw


def fixture():

    h = 504976
    blocks = h + 1

    minted = nw.expected_minted(blocks)

    return {
        "network": "mainnet",
        "ticker": "NIGHT",
        "genesis": nw.GENESIS,
        "protocol_version": 8,
        "wire_version": 6,
        "blocks": blocks,
        "tip_height": h,
        "tip": "a" * 64,
        "difficulty": 389996,
        "circulating": minted - 2521000000,
        "minted": minted,
        "burned_fees": 2521000000,
        "max_supply": 90000000,
        "peers": 55,
        "utxos": 214528,
        "kernels": 530187,
        "mempool": 0,
        "tip_time": 1794508830,
        "utxo_root": "b" * 64,
        "total_work": "107705507984",
        "supply_invariant_ok": True,
        "peer_versions": {
            "nightfalld/1.1.2": 49
        },
    }


class Tests(unittest.TestCase):

    def test_exact_integer_issuance(self):

        self.assertEqual(
            nw.expected_minted(504977),
            302986200000000,
        )

        self.assertEqual(
            nw.expected_minted(7500000),
            4500000000000000,
        )

        self.assertEqual(
            nw.expected_minted(225000000),
            8999999925000000,
        )

        self.assertEqual(
            nw.expected_minted(225000001),
            8999999925000000,
        )

    def test_valid(self):

        p = nw.sanitize(fixture())

        self.assertEqual(
            nw.review(
                p,
                now=p["tip_time"],
            )["level"],
            "OBSERVED",
        )

    def test_supply_forgery(self):

        p = fixture()
        p["minted"] += 1

        self.assertEqual(
            nw.review(
                nw.sanitize(p),
                now=p["tip_time"],
            )["level"],
            "CRITICAL",
        )

    def test_supply_arithmetic(self):

        p = fixture()
        p["circulating"] += 1

        self.assertEqual(
            nw.review(
                nw.sanitize(p),
                now=p["tip_time"],
            )["level"],
            "CRITICAL",
        )

    def test_block_count_forged(self):

        p = fixture()
        p["blocks"] += 1

        self.assertEqual(
            nw.review(
                nw.sanitize(p),
                now=p["tip_time"],
            )["level"],
            "CRITICAL",
        )

    def test_network_refusal(self):

        for key, value in (
            ("network", "devnet"),
            ("ticker", "OTHER"),
            ("genesis", "f" * 64),
        ):

            p = fixture()
            p[key] = value

            with self.assertRaises(ValueError):
                nw.sanitize(p)

    def test_bool_int_rejected(self):

        p = fixture()
        p["blocks"] = True

        with self.assertRaises(ValueError):
            nw.sanitize(p)

    def test_unexpected_data_never_persisted(self):

        p = fixture()

        p["secret_key"] = "REDACT_ME"
        p["ip_address"] = "127.0.0.1"

        p["peer_versions"][
            "127.0.0.1:17891"
        ] = 2

        encoded = json.dumps(
            nw.sanitize(p)
        )

        self.assertNotIn(
            "REDACT_ME", encoded
        )

        self.assertNotIn(
            "127.0.0.1", encoded
        )

    def test_bad_hash(self):

        p = fixture()
        p["tip"] = "Z" * 64

        with self.assertRaises(ValueError):
            nw.sanitize(p)

    def test_reorg_warning(self):

        a = nw.sanitize(fixture())

        b = dict(a)
        b["tip"] = "b" * 64

        self.assertEqual(
            nw.review(
                b,
                a,
                now=a["tip_time"],
            )["level"],
            "CAUTION",
        )

    def test_work_regression(self):

        a = nw.sanitize(fixture())
        b = dict(a)

        b["tip_height"] += 1
        b["blocks"] += 1
        b["minted"] += nw.REWARD
        b["circulating"] += nw.REWARD
        b["total_work"] = "1"

        self.assertEqual(
            nw.review(
                b,
                a,
                now=a["tip_time"],
            )["level"],
            "CRITICAL",
        )

    def test_old_versions(self):

        p = fixture()

        p["peer_versions"][
            "nightfalld/1.0.11"
        ] = 3

        self.assertEqual(
            nw.review(
                nw.sanitize(p),
                now=p["tip_time"],
            )["level"],
            "CAUTION",
        )

    def test_history_seal(self):

        p = nw.sanitize(fixture())

        rows = nw.seal([
            {
                "time_utc": "2026-11-12T12:00:00+00:00",
                "claim": p,
            },
        ])

        self.assertTrue(
            nw.validate_history(rows)
        )

        rows[0]["claim"]["tip_height"] += 1

        with self.assertRaises(ValueError):
            nw.validate_history(rows)

    def test_history_link(self):

        p = nw.sanitize(fixture())

        rows = nw.seal([
            {
                "time_utc": str(i),
                "claim": p,
            }
            for i in range(3)
        ])

        rows[1][
            "previous_sha256"
        ] = "f" * 64

        with self.assertRaises(ValueError):
            nw.validate_history(rows)

    def test_render_reports_and_no_js(self):

        p = nw.sanitize(fixture())

        rows = nw.seal([
            {
                "time_utc": "2026-11-12T12:00:00+00:00",
                "claim": p,
            },
        ])

        with tempfile.TemporaryDirectory() as d:

            with patch.object(
                nw,
                "OUT",
                Path(d),
            ):

                report = nw.make_reports(rows)

                self.assertFalse(
                    report["externally_verified"]
                )

                page = (
                    Path(d) / "report.html"
                ).read_text()

                self.assertNotIn(
                    "<script", page
                )

                self.assertIn(
                    "UNKNOWN", page
                )

                self.assertTrue(
                    (Path(d) / "report.md").exists()
                )

                self.assertTrue(
                    (Path(d) / "report.json").exists()
                )

    def test_unknown_pool_attribution_not_invented(self):

        p = nw.sanitize(fixture())

        self.assertNotIn(
            "pool_hashrate", p
        )

        self.assertNotIn(
            "addresses", p
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
