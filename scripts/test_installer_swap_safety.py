"""Exercise only the installers' swap lifecycle with mocked privileged tools.

No installer is executed, no real swap is enabled, and no 2 GiB file is made.
"""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INSTALLERS = (
    ROOT / "scripts/install-node-linux.sh",
    ROOT / "scripts/install-seed-node-linux.sh",
)


def swap_fragment(path):
    source = path.read_text(encoding="utf-8")
    begin = source.index('SWAPFILE=""')
    end = source.index('BUILD="$(mktemp -d)"', begin) + len('BUILD="$(mktemp -d)"')
    return source[begin:end]


class SwapLifecycleTests(unittest.TestCase):
    def run_case(self, installer, *, alloc_rc="0", mkswap_rc="0", swapon_rc="0", swapoff_rc="0", end_rc="0"):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            events = root / "events"
            # A file with the old fixed name must not be deleted or rewritten.
            old_file = root / "nightfall-build.swap"
            old_file.write_bytes(b"pre-existing operator data\n")
            for name in ("fallocate", "dd", "mkswap", "swapon", "swapoff"):
                executable = bin_dir / name
                executable.write_text(
                    '#!/bin/sh\n'
                    'printf "%s %s\\n" "$(basename "$0")" "$*" >> "$EVENTS"\n'
                    'case "$(basename "$0")" in\n'
                    '  fallocate) exit "$ALLOC_RC";;\n'
                    '  dd) exit 1;;\n'
                    '  mkswap) exit "$MKSWAP_RC";;\n'
                    '  swapon) exit "$SWAPON_RC";;\n'
                    '  swapoff) exit "$SWAPOFF_RC";;\n'
                    'esac\nexit 0\n', encoding="utf-8"
                )
                executable.chmod(0o700)
            fragment = swap_fragment(installer)
            self.assertNotIn("SWAP_PATH=/var/tmp/nightfall-build.swap", fragment)
            fragment = fragment.replace("/var/tmp/nightfall-build.", str(root) + "/nightfall-build.")
            command = (
                'set -euo pipefail\n'
                'TOTAL_MB=1024\n'
                'info() { :; }\n'
                'warn() { :; }\n'
                'die() { echo "$*" >&2; exit 1; }\n'
                + fragment + f'\nexit {end_rc}\n'
            )
            env = dict(os.environ, PATH=str(bin_dir) + os.pathsep + os.environ["PATH"],
                       TMPDIR=str(root), EVENTS=str(events), ALLOC_RC=alloc_rc,
                       MKSWAP_RC=mkswap_rc, SWAPON_RC=swapon_rc, SWAPOFF_RC=swapoff_rc)
            run = subprocess.run(["bash", "-c", command], env=env, capture_output=True, text=True)
            self.assertEqual(run.returncode, int(end_rc), run.stderr)
            self.assertEqual(old_file.read_bytes(), b"pre-existing operator data\n")
            leftovers = list(root.glob("nightfall-build.*.swap"))
            if swapoff_rc != "0":
                self.assertEqual(len(leftovers), 1, "active swap must remain named")
            else:
                self.assertEqual(leftovers, [], f"temporary swap left behind: {leftovers}")
            log = events.read_text() if events.exists() else ""
            if alloc_rc == "0" and mkswap_rc == "0" and swapon_rc == "0":
                self.assertIn("swapon ", log)
                self.assertIn("swapoff ", log)
            else:
                self.assertNotIn("swapoff ", log)

    def test_no_collision_success_cleanup(self):
        for path in INSTALLERS:
            with self.subTest(installer=path.name):
                self.run_case(path)

    def test_allocation_failure_cleanup(self):
        for path in INSTALLERS:
            with self.subTest(installer=path.name):
                self.run_case(path, alloc_rc="1")

    def test_mkswap_failure_cleanup(self):
        for path in INSTALLERS:
            with self.subTest(installer=path.name):
                self.run_case(path, mkswap_rc="1")

    def test_swapon_failure_cleanup(self):
        for path in INSTALLERS:
            with self.subTest(installer=path.name):
                self.run_case(path, swapon_rc="1")

    def test_failed_swapoff_preserves_active_file(self):
        for path in INSTALLERS:
            with self.subTest(installer=path.name):
                self.run_case(path, swapoff_rc="1")

    def test_exit_failure_still_cleans_up(self):
        for path in INSTALLERS:
            with self.subTest(installer=path.name):
                self.run_case(path, end_rc="7")


if __name__ == "__main__":
    unittest.main()
