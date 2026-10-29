#!/usr/bin/env python3
"""
Offline adversarial continuity regression tests.

No live node.
No wallet.
No mining.
No external network.
"""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).with_name(
    "node_continuity.py"
)

SPEC = importlib.util.spec_from_file_location(
    "nightfall_continuity",
    SCRIPT
)

mod = importlib.util.module_from_spec(
    SPEC
)

SPEC.loader.exec_module(
    mod
)

GENESIS = (
    "061a052d49607ff8f4b306c75d622eb"
    "d230cff4ec3a45a6dffc2f7738d4b20de"
)


def evidence(
    height=100,
    work="1000",
    tip="a" * 64,
    warnings=()
):
    return {
        "schema":
            "nightfall-node-evidence-v1",
        "samples": [{
            "tip_height": height,
            "total_work": work,
            "tip": tip,
            "network": "mainnet",
            "genesis": GENESIS,
            "protocol_version": 8,
            "wire_version": 6
        }],
        "warnings": list(warnings)
    }


class ContinuityTests(unittest.TestCase):

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()

        self.addCleanup(
            self.temp.cleanup
        )

        self.path = (
            Path(self.temp.name)
            / "separate"
            / "anchor.json"
        )

    def test_baseline_and_progress(self):
        a = mod.evaluate(
            evidence(),
            self.path
        )

        self.assertEqual(
            "BASELINED",
            a["status"]
        )

        self.assertTrue(
            a["state_written"]
        )

        b = mod.evaluate(
            evidence(),
            self.path
        )

        self.assertEqual(
            "UNCHANGED",
            b["status"]
        )

        c = mod.evaluate(
            evidence(
                101,
                "1100",
                "b" * 64
            ),
            self.path
        )

        self.assertEqual(
            "ADVANCED",
            c["status"]
        )

        self.assertEqual(
            101,
            mod.load_anchor(
                self.path
            )["height"]
        )

    def test_height_rollback_keeps_last_good(self):
        mod.evaluate(
            evidence(),
            self.path
        )

        result = mod.evaluate(
            evidence(
                99,
                "1100",
                "b" * 64
            ),
            self.path
        )

        self.assertEqual(
            "CAUTION",
            result["status"]
        )

        self.assertIn(
            "height decreased: possible reorg or rollback",
            result["alerts"]
        )

        self.assertFalse(
            result["state_written"]
        )

        self.assertEqual(
            100,
            mod.load_anchor(
                self.path
            )["height"]
        )

    def test_work_rollback_and_conflict(self):
        mod.evaluate(
            evidence(),
            self.path
        )

        samples = (
            evidence(
                101,
                "999",
                "b" * 64
            ),
            evidence(
                100,
                "1001",
                "c" * 64
            ),
            evidence(
                101,
                "1000",
                "b" * 64
            ),
            evidence(
                100,
                "1011",
                "a" * 64
            )
        )

        for sample in samples:
            with self.subTest(
                sample=sample
            ):
                result = mod.evaluate(
                    sample,
                    self.path
                )

                self.assertEqual(
                    "CAUTION",
                    result["status"]
                )

                self.assertEqual(
                    100,
                    mod.load_anchor(
                        self.path
                    )["height"]
                )

    def test_warning_never_advances(self):
        result = mod.evaluate(
            evidence(
                warnings=[
                    "no live peers"
                ]
            ),
            self.path
        )

        self.assertEqual(
            "CAUTION",
            result["status"]
        )

        self.assertFalse(
            self.path.exists()
        )

        mod.evaluate(
            evidence(),
            self.path
        )

        result = mod.evaluate(
            evidence(
                101,
                "1100",
                "b" * 64,
                [
                    "peer ahead of local tip"
                ]
            ),
            self.path
        )

        self.assertEqual(
            "CAUTION",
            result["status"]
        )

        self.assertEqual(
            100,
            mod.load_anchor(
                self.path
            )["height"]
        )

    def test_corrupted_checksum(self):
        mod.evaluate(
            evidence(),
            self.path
        )

        obj = json.loads(
            self.path.read_text()
        )

        obj["anchor"]["height"] = 9999

        self.path.write_text(
            json.dumps(obj)
        )

        with self.assertRaisesRegex(
            mod.GuardError,
            "integrity mismatch"
        ):
            mod.evaluate(
                evidence(
                    101,
                    "1200",
                    "b" * 64
                ),
                self.path
            )

    def test_oversized_and_duplicate_json(self):
        mod.evaluate(
            evidence(),
            self.path
        )

        self.path.write_bytes(
            b"a" * (mod.LIMIT + 1)
        )

        with self.assertRaises(
            mod.GuardError
        ):
            mod.load_anchor(
                self.path
            )

        self.path.write_bytes(
            b'{"a":1,"a":2}'
        )

        with self.assertRaisesRegex(
            mod.GuardError,
            "duplicate"
        ):
            mod.load_anchor(
                self.path
            )

    def test_symlink_is_rejected(self):
        backing = (
            Path(self.temp.name)
            / "backing.json"
        )

        backing.write_text(
            "keep me"
        )

        self.path.parent.mkdir(
            parents=True
        )

        self.path.symlink_to(
            backing
        )

        with self.assertRaises(
            mod.GuardError
        ):
            mod.evaluate(
                evidence(),
                self.path
            )

        self.assertEqual(
            "keep me",
            backing.read_text()
        )

    def test_dry_run(self):
        result = mod.evaluate(
            evidence(),
            self.path,
            dry_run=True
        )

        self.assertEqual(
            "BASELINED",
            result["status"]
        )

        self.assertFalse(
            self.path.exists()
        )

    def test_real_receipt_validation(self):
        kit = mod.evidence()

        status = dict(
            network="mainnet",
            genesis=GENESIS,
            protocol_version=8,
            wire_version=6,
            tip="a" * 64,
            tip_height=100,
            blocks=101,
            total_work="1000",
            minted=1000000000,
            burned_fees=100,
            circulating=999999900,
            loading=False,
            supply_invariant_ok=True,
            live_peers=2,
            best_peer_height=100,
            stalled_on_fork=False,
            reorg_in_flight=False,
            mining=False,
            mining_idle=""
        )

        status = kit.check_status(
            status
        )

        body = {
            "schema":
                "nightfall-node-evidence-v1",
            "scope":
                "untrusted-local-rpc-claims-not-consensus-proof",
            "samples": [
                status,
                status
            ],
            "checks": [
                "status",
                "verify_supply"
            ],
            "assessment": "OBSERVED",
            "warnings": []
        }

        report = kit.seal(
            body
        )

        receipt = (
            Path(self.temp.name)
            / "receipt.json"
        )

        receipt.write_text(
            json.dumps(report)
        )

        self.assertEqual(
            0,
            mod.main([
                "--state",
                str(self.path),
                "--receipt",
                str(receipt)
            ])
        )

        report["payload"]["samples"][1][
            "live_peers"
        ] = 0

        receipt.write_text(
            json.dumps(report)
        )

        self.assertEqual(
            2,
            mod.main([
                "--state",
                str(self.path),
                "--receipt",
                str(receipt)
            ])
        )

        self.assertEqual(
            100,
            mod.load_anchor(
                self.path
            )["height"]
        )


if __name__ == "__main__":
    unittest.main(
        verbosity=2
    )
