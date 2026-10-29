#!/usr/bin/env python3
"""Offline adversarial regression tests."""

import copy
import json
import unittest

from public_network_health import (
    GENESIS,
    InvalidPublicData,
    build_report,
    clean,
    digest,
    evaluate,
    read_json,
)

BASE = {
    "network": "mainnet",
    "genesis": GENESIS,
    "protocol_version": 8,
    "wire_version": 6,
    "blocks": 101,
    "tip_height": 100,
    "tip": "a" * 64,
    "tip_time": 1793290000,
    "difficulty": 2000,
    "total_work": "400000",
    "minted": 60600000000,
    "burned_fees": 100000,
    "circulating": 60599900000,
    "max_supply": 90000000,
    "supply_invariant_ok": True,
    "loading": False,
    "peers": 2,
    "peer_versions": {
        "nightfalld/1.0.12": 2
    },
}


class PublicNetworkHealthTest(unittest.TestCase):

    def sample(self, modified=None):
        data = copy.deepcopy(BASE)

        if modified:
            data.update(modified)

        return {
            "observed_unix": 1793290010,
            "public": clean(data),
        }

    def test_valid(self):
        self.assertEqual(
            clean(BASE)["tip_height"],
            100,
        )

    def test_extraneous_private_fields_redacted(self):
        data = copy.deepcopy(BASE)

        data["wallet_seed"] = (
            "SHOULD_NEVER_APPEAR"
        )

        data["peer_ips"] = [
            "192.0.2.12"
        ]

        report = build_report(
            [{
                "observed_unix": 1793290010,
                "public": clean(data),
            }],
            1793290010,
        )

        text = json.dumps(report)

        self.assertNotIn(
            "SHOULD_NEVER_APPEAR",
            text,
        )

        self.assertNotIn(
            "192.0.2.12",
            text,
        )

        hashvalue = report.pop(
            "sha256"
        )

        self.assertEqual(
            hashvalue,
            digest(report),
        )

    def test_duplicate_json_keys_rejected(self):
        with self.assertRaises(
            InvalidPublicData
        ):
            read_json(
                b'{"network":"mainnet","network":"devnet"}'
            )

    def test_nonfinite_and_oversize(self):
        for body in (
            b'{"bad":NaN}',
            b' ' * 65537,
        ):
            with self.assertRaises(
                InvalidPublicData
            ):
                read_json(body)

    def test_monetary_inconsistency_rejected(self):
        with self.assertRaises(
            InvalidPublicData
        ):
            clean({
                **BASE,
                "circulating": (
                    BASE["circulating"] + 1
                ),
            })

    def test_bool_is_not_valid_height(self):
        with self.assertRaises(
            InvalidPublicData
        ):
            clean({
                **BASE,
                "tip_height": True,
            })

    def test_network_and_protocol_identity(self):
        for changes in (
            {"network": "devnet"},
            {"genesis": "f" * 64},
            {"protocol_version": 7},
            {"wire_version": 5},
        ):
            with self.subTest(
                changes=changes
            ):
                with self.assertRaises(
                    InvalidPublicData
                ):
                    clean({
                        **BASE,
                        **changes,
                    })

    def test_invalid_version_label_rejected(self):
        with self.assertRaises(
            InvalidPublicData
        ):
            clean({
                **BASE,
                "peer_versions": {
                    "10.0.0.1": 1
                },
            })

    def test_version_aggregates_are_not_peers(self):
        s = self.sample({
            "peers": 1,
            "peer_versions": {
                "nightfalld/1.0.11": 200
            },
        })

        self.assertEqual(
            s["public"]["peer_versions"][
                "nightfalld/1.0.11"
            ],
            200,
        )

    def test_reported_supply_failure(self):
        s = self.sample({
            "supply_invariant_ok": False
        })

        self.assertIn(
            "REPORTED_SUPPLY_CHECK_FALSE",
            evaluate(
                [s],
                1793290010,
            ),
        )

    def test_stale_and_future_tips(self):
        self.assertIn(
            "TIP_OLDER_THAN_15_MINUTES",
            evaluate(
                [self.sample()],
                1793291000,
            ),
        )

        self.assertIn(
            "TIP_TIMESTAMP_IN_FUTURE",
            evaluate(
                [self.sample()],
                1793289800,
            ),
        )

    def test_work_regression(self):
        a = self.sample()

        b = self.sample({
            "total_work": "399999"
        })

        self.assertIn(
            "REPORTED_WORK_REGRESSION",
            evaluate(
                [a, b],
                1793290010,
            ),
        )

    def test_shorter_heavier_chain_is_not_work_regression(self):
        a = self.sample()

        b = self.sample({
            "blocks": 100,
            "tip_height": 99,
            "tip": "b" * 64,
            "total_work": "410000",
        })

        warnings = evaluate(
            [a, b],
            1793290010,
        )

        self.assertIn(
            "REPORTED_HEIGHT_ROLLBACK_CHECK_REORG",
            warnings,
        )

        self.assertNotIn(
            "REPORTED_WORK_REGRESSION",
            warnings,
        )

    def test_tip_hash_change_same_height(self):
        a = self.sample()

        b = self.sample({
            "tip": "b" * 64
        })

        self.assertIn(
            "REPORTED_SAME_HEIGHT_TIP_CHANGE",
            evaluate(
                [a, b],
                1793290010,
            ),
        )

    def test_missing_field(self):
        data = copy.deepcopy(BASE)

        del data["minted"]

        with self.assertRaises(
            InvalidPublicData
        ):
            clean(data)


if __name__ == "__main__":
    unittest.main(
        verbosity=2
    )
