import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "pr100_production_gate",
    Path(__file__).with_name(
        "pr100_production_gate.py"
    )
)

mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


class EvidencePolicy(unittest.TestCase):

    def test_same_length_mutation_detected(self):
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / "chain.bin"

            f.write_bytes(b"AAAA")
            first = mod.fingerprint(tmp)

            f.write_bytes(b"AAAB")
            second = mod.fingerprint(tmp)

            self.assertNotEqual(
                first["sha256_tree"],
                second["sha256_tree"]
            )
            self.assertEqual(
                first["bytes"],
                second["bytes"]
            )

    def test_empty_fixture_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError):
                mod.fingerprint(tmp)

    def test_symlink_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            (root / "real").mkdir()
            (root / "real" / "x").write_bytes(b"x")

            (root / "link").symlink_to(
                root / "real",
                target_is_directory=True
            )

            with self.assertRaises(ValueError):
                mod.fingerprint(root / "link")

    def test_real_marker_required(self):
        self.assertIsNone(
            mod.parse_mainnet_pass(
                "test result: ok. 1 passed;"
            )
        )

        self.assertIsNone(
            mod.parse_mainnet_pass(
                "PR100_REAL_MAINNET_PASS "
                "checkpoint=25000 catchup=100"
            )
        )

        line = (
            "PR100_REAL_MAINNET_PASS "
            "checkpoint=505000 "
            "source_tip=505005 "
            "catchup=5 "
            "snapshot_bytes=1000 "
            "load_s=1.00 "
            "export_s=2.00 "
            "import_s=3.00 "
            "catchup_s=4.00 "
            "peak_ram_kib=Some(1500)"
        )

        parsed = mod.parse_mainnet_pass(line)

        self.assertEqual(
            parsed["checkpoint"],
            "505000"
        )

    def test_missing_data_cannot_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            a = mod.check_mainnet(
                root,
                root / "missing",
                root
            )

            b = mod.check_legacy(
                root,
                root / "missing",
                root
            )

            self.assertEqual(a["state"], "BLOCKED")
            self.assertEqual(b["state"], "BLOCKED")


class EvidenceConsistencyV2(unittest.TestCase):
    # pr100-evidence-consistency-v2: no trusted claims from malformed logs.

    @staticmethod
    def receipt():
        return (
            'PR100_REAL_MAINNET_PASS checkpoint=505000 '
            'source_tip=505005 catchup=5 snapshot_bytes=1000 '
            'load_s=1.00 export_s=2.00 import_s=3.00 '
            'catchup_s=4.00 peak_ram_kib=Some(1500)'
        )

    def test_consistent_receipt_accepted(self):
        self.assertEqual(mod.parse_mainnet_pass(self.receipt())['catchup'], '5')

    def test_contradictory_or_forged_receipts_rejected(self):
        good = self.receipt()
        changes = (
            good.replace('catchup=5', 'catchup=6'),
            good.replace('source_tip=505005', 'source_tip=505000'),
            good.replace('checkpoint=505000', 'checkpoint=505000abc'),
            good.replace('checkpoint=505000', 'checkpoint=25000'),
            good.replace('snapshot_bytes=1000', 'snapshot_bytes=0'),
            good.replace('snapshot_bytes=1000', 'snapshot_bytes=536870913'),
            good.replace('load_s=1.00', 'load_s=-1.00'),
            good.replace('load_s=1.00', 'load_s=1e999'),
            good.replace('peak_ram_kib=Some(1500)', 'peak_ram_kib=Some(-1)'),
            good + ' checkpoint=505000',
            good + ' unrelated=1',
            'log: ' + good,
            good + '\n' + good,
            good.replace('source_tip=505005', 'source_tip=505005.0'),
            good.replace('export_s=2.00', 'export_s=inf'),
            good.replace('catchup_s=4.00', 'catchup_s=NaN'),
        )
        for bad in changes:
            with self.subTest(receipt=bad[:80]):
                self.assertIsNone(mod.parse_mainnet_pass(bad))


class FixtureAttestationFailClosed(unittest.TestCase):
    # pr100-fingerprint-failclosed-v1

    def test_permission_error_during_walk_is_not_ignored(self):
        from unittest import mock

        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "block.dat").write_bytes(b"abcd")

            def denied(*args, **kwargs):
                kwargs["onerror"](PermissionError("unreadable subtree"))
                return iter(())

            with mock.patch.object(mod.os, "walk", side_effect=denied):
                with self.assertRaises(OSError):
                    mod.fingerprint(tmp)

    def test_empty_directory_changes_tree_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "block.dat").write_bytes(b"abcd")

            a = mod.fingerprint(root)

            (root / "new-empty-dir").mkdir()

            b = mod.fingerprint(root)

            self.assertNotEqual(a["sha256_tree"], b["sha256_tree"])
            self.assertEqual(a["bytes"], b["bytes"])

    def test_same_length_path_replacement_during_open_rejected(self):
        import os
        from unittest import mock

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fixture = root / "fixture"
            fixture.mkdir()

            target = fixture / "blocks.dat"
            target.write_bytes(b"AAAA")

            replacement = root / "replacement"
            replacement.write_bytes(b"BBBB")

            original_open = os.open

            def swapping_open(path, flags, *args, **kwargs):
                if Path(path) == target:
                    os.replace(replacement, target)

                return original_open(path, flags, *args, **kwargs)

            with mock.patch.object(mod.os, "open", side_effect=swapping_open):
                with self.assertRaises(ValueError):
                    mod.fingerprint(fixture)

    def test_symlink_and_fifo_rejected(self):
        import os

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "block.dat").write_bytes(b"abcd")

            (root / "link").symlink_to(root / "block.dat")

            with self.assertRaises(ValueError):
                mod.fingerprint(root)

            (root / "link").unlink()

            if hasattr(os, "mkfifo"):
                os.mkfifo(root / "pipe")

                with self.assertRaises(ValueError):
                    mod.fingerprint(root)


if __name__ == "__main__":
    unittest.main()
