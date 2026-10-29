import json
import tempfile
import unittest

from pathlib import Path

import termux_restart_guard as g


class TestRestartGuard(unittest.TestCase):

    def setUp(self):

        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)

        self.root = Path(self.temp.name)

        (
            self.root / "blocks.bin"
        ).write_bytes(b"12345678")

        (
            self.root / "install-id"
        ).write_text("local-test-id")

        self.meta = dict(
            network="mainnet",
            protocol_version=8,
            genesis_hash=g.GENESIS,
            block_count=5,
            validated_tip="a" * 64,
            validated_bytes=8,
            validated_by="local-test-id"
        )

        self.save()

    def save(self):

        (
            self.root / "chain-meta.json"
        ).write_text(
            json.dumps(self.meta)
        )

    def check(self, mining=False):

        return g.inspect(
            self.root,
            mining
        )["result"]

    def test_clean(self):

        self.assertEqual(
            self.check(),
            "READY"
        )

    def test_genesis(self):

        self.meta["genesis_hash"] = "0" * 64
        self.save()

        self.assertEqual(
            self.check(),
            "BLOCKED"
        )

    def test_network(self):

        self.meta["network"] = "devnet"
        self.save()

        self.assertEqual(
            self.check(),
            "BLOCKED"
        )

    def test_protocol(self):

        self.meta["protocol_version"] = 7
        self.save()

        self.assertEqual(
            self.check(),
            "BLOCKED"
        )

    def test_boolean_protocol(self):

        self.meta["protocol_version"] = True
        self.save()

        self.assertEqual(
            self.check(),
            "BLOCKED"
        )

    def test_genesis_only(self):

        self.meta["block_count"] = 1
        self.save()

        self.assertEqual(
            self.check(),
            "BLOCKED"
        )

    def test_missing_chain(self):

        (
            self.root / "blocks.bin"
        ).unlink()

        self.assertEqual(
            self.check(),
            "BLOCKED"
        )

    def test_recovery_journal(self):

        (
            self.root / ".recovery-journal.json"
        ).write_text("{}")

        self.assertEqual(
            self.check(),
            "BLOCKED"
        )

    def test_size_mismatch(self):

        self.meta["validated_bytes"] = 99
        self.save()

        self.assertEqual(
            self.check(),
            "REVIEW"
        )

    def test_foreign_validation_record(self):

        self.meta["validated_by"] = "someone-else"
        self.save()

        self.assertEqual(
            self.check(),
            "REVIEW"
        )

    def test_missing_miner_seed(self):

        self.assertEqual(
            self.check(True),
            "BLOCKED"
        )

    def test_present_miner_seed(self):

        f = self.root / "miner.seed"

        f.write_bytes(b"test only")
        f.chmod(0o600)

        self.assertEqual(
            self.check(True),
            "READY"
        )

    def test_public_miner_seed(self):

        f = self.root / "miner.seed"

        f.write_bytes(b"test only")
        f.chmod(0o644)

        self.assertEqual(
            self.check(True),
            "REVIEW"
        )

    def test_symlink_chain(self):

        (
            self.root / "blocks.bin"
        ).unlink()

        (
            self.root / "blocks.bin"
        ).symlink_to(
            self.root / "install-id"
        )

        self.assertEqual(
            self.check(),
            "BLOCKED"
        )

    def test_oversized_metadata(self):

        (
            self.root / "chain-meta.json"
        ).write_bytes(
            b"a" * 70000
        )

        self.assertEqual(
            self.check(),
            "BLOCKED"
        )

    def test_no_file_changes(self):

        before = {
            p.name: p.read_bytes()
            for p in self.root.iterdir()
        }

        g.inspect(self.root)

        after = {
            p.name: p.read_bytes()
            for p in self.root.iterdir()
        }

        self.assertEqual(
            before,
            after
        )


if __name__ == "__main__":
    unittest.main()
