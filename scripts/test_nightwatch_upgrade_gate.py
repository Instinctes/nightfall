#!/usr/bin/env python3
"""Offline tests for Nightwatch upgrade evidence."""

import tempfile
from pathlib import Path
import unittest

import nightwatch as nw
import nightwatch_upgrade_gate as gate


def claim(height=505164, labels=None):

    if labels is None:
        labels = {
            "nightfalld/0.7.2": 2,
            "nightfalld/0.8.0": 5,
            "nightfalld/1.0.11": 1,
            "nightfalld/1.0.6": 1,
            "nightfalld/1.1.1": 12,
            "nightfalld/1.1.2": 49,
        }

    return {
        "tip_height": height,
        "tip": "a" * 64,
        "peers": 70,
        "peer_versions": labels,
    }


def history(claims):

    return nw.seal([
        {
            "time_utc": time,
            "claim": p,
        }
        for time, p in claims
    ])


class UpgradeGateTests(unittest.TestCase):

    def test_nine_legacy_labels(self):

        rows = history([
            (
                "2026-11-12T19:28:13+00:00",
                claim(),
            )
        ])

        result = gate.evaluate(rows)

        self.assertEqual(
            result["reported_legacy_labels"],
            9,
        )

        self.assertEqual(
            result["reported_compatible_labels"],
            61,
        )

    def test_one_one_one_compatible(self):

        result = gate.classify({
            "nightfalld/1.1.1": 12,
        })

        self.assertEqual(
            result["legacy"],
            0,
        )

    def test_old_versions_legacy(self):

        result = gate.classify({
            "nightfalld/1.0.11": 1,
        })

        self.assertEqual(
            result["legacy"],
            1,
        )

    def test_reject_negative_count(self):

        with self.assertRaises(ValueError):

            gate.classify({
                "nightfalld/1.0.11": -1,
            })

    def test_reject_invalid_history_seal(self):

        rows = history([
            (
                "2026-11-12T19:28:13+00:00",
                claim(),
            )
        ])

        rows[0]["claim"]["tip_height"] += 1

        with self.assertRaises(ValueError):
            gate.evaluate(rows)

    def test_persistence_requires_time(self):

        rows = history([
            (
                "2026-11-12T19:28:13+00:00",
                claim(),
            ),
            (
                "2026-11-12T19:29:13+00:00",
                claim(),
            ),
        ])

        self.assertFalse(
            gate.evaluate(rows)[
                "persistent_legacy_labels"
            ]
        )

    def test_persistence_after_ten_minutes(self):

        rows = history([
            (
                "2026-11-12T19:28:13+00:00",
                claim(),
            ),
            (
                "2026-11-12T19:40:13+00:00",
                claim(),
            ),
        ])

        self.assertTrue(
            gate.evaluate(rows)[
                "persistent_legacy_labels"
            ]
        )

    def test_report_files(self):

        rows = history([
            (
                "2026-11-12T19:28:13+00:00",
                claim(),
            )
        ])

        result = gate.evaluate(rows)

        with tempfile.TemporaryDirectory() as d:

            gate.write_evidence(
                result,
                Path(d),
            )

            self.assertTrue(
                (Path(d) / "upgrade-gate.md").exists()
            )

            self.assertTrue(
                (Path(d) / "upgrade-gate.json").exists()
            )

    def test_clean_latest(self):

        rows = history([
            (
                "2026-11-12T19:28:13+00:00",
                claim(
                    labels={
                        "nightfalld/1.1.2": 49,
                    }
                ),
            )
        ])

        self.assertEqual(
            gate.evaluate(rows)["status"],
            "NO_LEGACY_LABELS_IN_LAST_SAMPLE",
        )

    def test_future_version_not_assumed_safe(self):

        result = gate.classify({
            "nightfalld/2.0.0": 7,
        })

        self.assertEqual(
            result["compatible"],
            0,
        )

        self.assertEqual(
            result["unclassified"],
            7,
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
