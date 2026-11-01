#!/usr/bin/env python3
"""Deterministic offline Nightfall upgrade gate tests."""

import sys
import unittest
from pathlib import Path

sys.path.insert(
    0,
    str(Path(__file__).resolve().parent)
)

import nightfall_upgrade_gate as gate


class GateTests(unittest.TestCase):
    NOW = 1_800_000_000

    def fixture(self):
        blocks = 441_746
        minted = blocks * gate.INITIAL_SUBSIDY_DARK
        burned = 2_219_400_000

        return {
            "network": "mainnet",
            "genesis": gate.GENESIS,
            "protocol_version": 8,
            "wire_version": 6,
            "supply_invariant_ok": True,
            "loading": False,
            "max_supply": gate.HARD_CAP_NIGHT,
            "blocks": blocks,
            "tip_height": blocks - 1,
            "minted": minted,
            "burned_fees": burned,
            "circulating": minted - burned,
            "tip_time": self.NOW - 15,
            "peer_versions": {
                "nightfalld/1.1.1": 3,
                "nightfalld/1.1.2+gpu.1": 1
            }
        }

    def test_clean_sample(self):
        result = gate.evaluate(
            self.fixture(),
            now=self.NOW
        )

        self.assertEqual(
            result["state"], "PASS"
        )

        self.assertEqual(
            result["blocks_remaining"], 58_255
        )

        self.assertEqual(
            result["observed_peer_version_sample"]["ready"],
            4
        )

    def test_old_software_warns(self):
        data = self.fixture()

        data["peer_versions"]["nightfalld/1.0.12"] = 2

        result = gate.evaluate(
            data,
            now=self.NOW
        )

        self.assertEqual(
            result["state"], "WARN"
        )

        self.assertEqual(
            result["observed_peer_version_sample"]["legacy"],
            2
        )

    def test_unknown_versions_warn(self):
        data = self.fixture()

        data["peer_versions"]["unparsable"] = 1

        self.assertEqual(
            gate.evaluate(data, now=self.NOW)["state"],
            "WARN"
        )

    def test_wrong_genesis_fails(self):
        data = self.fixture()
        data["genesis"] = "wrong"

        self.assertEqual(
            gate.evaluate(data, now=self.NOW)["state"],
            "FAIL"
        )

    def test_invalid_supply_flag_fails(self):
        data = self.fixture()
        data["supply_invariant_ok"] = False

        self.assertEqual(
            gate.evaluate(data, now=self.NOW)["state"],
            "FAIL"
        )

    def test_mint_mismatch_fails(self):
        data = self.fixture()
        data["minted"] += 1

        self.assertEqual(
            gate.evaluate(data, now=self.NOW)["state"],
            "FAIL"
        )

    def test_negative_counts_fail(self):
        data = self.fixture()
        data["blocks"] = -1

        self.assertEqual(
            gate.evaluate(data, now=self.NOW)["state"],
            "FAIL"
        )

    def test_stale_tip_fails(self):
        data = self.fixture()
        data["tip_time"] = self.NOW - 3600

        self.assertEqual(
            gate.evaluate(data, now=self.NOW)["state"],
            "FAIL"
        )

    def test_future_tip_fails(self):
        data = self.fixture()
        data["tip_time"] = self.NOW + 121

        self.assertEqual(
            gate.evaluate(data, now=self.NOW)["state"],
            "FAIL"
        )

    def test_invalid_peer_count_fails(self):
        data = self.fixture()

        data["peer_versions"]["nightfalld/1.1.2"] = True

        self.assertEqual(
            gate.evaluate(data, now=self.NOW)["state"],
            "FAIL"
        )

    def test_missing_data_fails_closed(self):
        data = self.fixture()
        del data["minted"]

        self.assertEqual(
            gate.evaluate(data, now=self.NOW)["state"],
            "FAIL"
        )

    def test_version_comparison(self):
        self.assertEqual(
            gate.version_tuple(
                "nightfalld/1.1.2+gpu.1"
            ),
            (1, 1, 2)
        )

        self.assertIsNone(
            gate.version_tuple(
                "nightfalld/random"
            )
        )


if __name__ == "__main__":
    unittest.main()
