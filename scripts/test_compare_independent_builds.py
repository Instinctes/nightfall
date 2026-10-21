"""Offline adversarial test cases; no download, compilation, keys or network."""
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("comparator", ROOT / "compare_independent_builds.py")
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


def receipt(**changes):
    r = {"schema": mod.SCHEMA, "source_commit": "a" * 64,
         "cargo_lock_sha256": "b" * 64,
         "rustc": "rustc 1.96.0 (mock)", "cargo": "cargo 1.96.0 (mock)",
         "host": "aarch64-linux-android", "profile": "release",
         "artifacts": {"nightfalld": {"sha256": "c" * 64, "bytes": 100}}}
    r.update(changes)
    return r


class IndependentBuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = Path(self.temp.name)

    def write(self, name, content):
        p = self.folder / name
        p.write_text(content)
        return p

    def test_identical_binary_receipts(self):
        self.assertEqual(mod.compare(receipt(), receipt()), ("MATCHING_BYTES", 0, []))

    def test_different_binary_bytes(self):
        r = receipt(artifacts={"nightfalld": {"sha256": "d" * 64, "bytes": 100}})
        self.assertEqual(mod.compare(receipt(), r), ("DIFFERENT_BYTES", 1, ["nightfalld"]))

    def test_same_hash_different_size_detected(self):
        r = receipt(artifacts={"nightfalld": {"sha256": "c" * 64, "bytes": 101}})
        self.assertEqual(mod.compare(receipt(), r)[0], "DIFFERENT_BYTES")

    def test_toolchain_or_lock_drift_not_comparable(self):
        for change in ({"rustc": "rustc 1.95.0 (mock)"},
                       {"cargo_lock_sha256": "f" * 64},
                       {"host": "x86_64-unknown-linux-gnu"},
                       {"source_commit": "e" * 64}):
            with self.subTest(change=change):
                self.assertEqual(mod.compare(receipt(), receipt(**change))[0], "INCOMPARABLE")

    def test_artifact_set_drift(self):
        a = receipt(artifacts={"nightfall-wallet": {"sha256": "c" * 64, "bytes": 100}})
        self.assertEqual(mod.compare(receipt(), a), ("INCOMPARABLE", 2, ["artifact_names"]))

    def test_unknown_metadata_rejected_for_privacy(self):
        with self.assertRaises(mod.BuildEvidenceError):
            mod.validate(receipt(seed="secret"))

    def test_boolean_byte_size_rejected(self):
        with self.assertRaises(mod.BuildEvidenceError):
            mod.validate(receipt(artifacts={"nightfalld": {"sha256": "c" * 64, "bytes": True}}))

    def test_invalid_hash_rejected(self):
        with self.assertRaises(mod.BuildEvidenceError):
            mod.validate(receipt(source_commit="a" * 63 + "Z"))

    def test_duplicate_json_fields_rejected(self):
        p = self.write("duplicate.json", '{"schema":"one","schema":"two"}')
        with self.assertRaisesRegex(mod.BuildEvidenceError, "duplicate"):
            mod.read_receipt(p)

    def test_oversized_receipt_rejected(self):
        p = self.write("large.json", " " * (mod.MAX_RECEIPT + 1))
        with self.assertRaisesRegex(mod.BuildEvidenceError, "oversized"):
            mod.read_receipt(p)

    def test_symlink_receipt_and_binary_rejected(self):
        p = self.write("original", "abc")
        alias = self.folder / "alias"
        alias.symlink_to(p)
        with self.assertRaises(mod.BuildEvidenceError):
            mod.read_receipt(alias)
        with self.assertRaises(mod.BuildEvidenceError):
            mod.digest_file(alias)

    def test_invalid_numeric_constant_rejected(self):
        p = self.write("nan.json", json.dumps(receipt()).replace('"bytes": 100', '"bytes": NaN'))
        with self.assertRaises(mod.BuildEvidenceError):
            mod.read_receipt(p)

    def test_record_reads_real_files_without_modifying(self):
        repo = self.folder / "repo"
        repo.mkdir()
        (repo / "Cargo.lock").write_bytes(b"test lock")
        binary = self.folder / "nightfalld"
        binary.write_bytes(b"binary test")
        def cmd(args, cwd=None):
            if args[1:] == ["rev-parse", "HEAD"]:
                return "a" * 64
            if args[1:] == ["status", "--porcelain", "--untracked-files=normal"]:
                return ""
            if args == ["rustc", "-vV"]:
                return "rustc 1.96.0 (mock)\nhost: aarch64-linux-android"
            if args == ["cargo", "--version"]:
                return "cargo 1.96.0 (mock)"
            raise AssertionError(args)
        with patch.object(mod, "command", side_effect=cmd):
            r = mod.record(repo, ["nightfalld=" + str(binary)])
        self.assertEqual(r["artifacts"]["nightfalld"]["bytes"], 11)
        self.assertEqual(r["profile"], "release")
        self.assertEqual(list(repo.iterdir()), [repo / "Cargo.lock"])

    def test_dirty_repo_fails_closed(self):
        repo = self.folder / "repo"
        repo.mkdir()
        (repo / "Cargo.lock").write_bytes(b"test lock")
        with patch.object(mod, "command", side_effect=["a" * 64, " M src/main.rs"]):
            with self.assertRaisesRegex(mod.BuildEvidenceError, "not clean"):
                mod.record(repo, ["nightfalld=unused"])

    def test_wrong_or_duplicate_binary_role(self):
        for bad in (["miner=abc"], ["nightfalld=abc", "nightfalld=def"], ["nightfalld"]):
            with self.subTest(bad=bad):
                repo = self.folder / "repo"
                repo.mkdir(exist_ok=True)
                (repo / "Cargo.lock").write_bytes(b"test lock")
                with patch.object(mod, "command", side_effect=["a" * 64, "", "rustc 1.96.0 (mock)\nhost: aarch64-linux-android", "cargo 1.96.0 (mock)"]):
                    with self.assertRaises(mod.BuildEvidenceError):
                        mod.record(repo, bad)

    def test_cli_diff_exit_code(self):
        p1 = self.write("one.json", json.dumps(receipt()))
        p2 = self.write("two.json", json.dumps(receipt(artifacts={"nightfalld": {"sha256": "d" * 64, "bytes": 100}})))
        proc = subprocess.run([sys.executable, str(ROOT / "compare_independent_builds.py"),
                               "compare", str(p1), str(p2)], capture_output=True, text=True)
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(json.loads(proc.stdout)["assessment"], "DIFFERENT_BYTES")

    def test_help_no_side_effects(self):
        p = subprocess.run([sys.executable, str(ROOT / "compare_independent_builds.py"), "--help"],
                           capture_output=True, text=True)
        self.assertEqual(p.returncode, 0)
        self.assertIn("compare", p.stdout)


if __name__ == "__main__":
    unittest.main()
