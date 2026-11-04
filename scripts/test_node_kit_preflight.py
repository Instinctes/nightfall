"""Offline adversarial and regression tests; no sudo, node, mining, or network."""
import importlib.util
import io
import json
import subprocess
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("node_kit_preflight", ROOT / "node_kit_preflight.py")
pre = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pre)


def host(**overrides):
    result = {
        "linux": True, "systemd_running": True, "arch": "x86_64", "root": True,
        "commands": {k: True for k in
                     ("bash", "systemctl", "sudo", "git", "cargo", "apt-get", "dnf")},
        "free_bytes": 8 * 1024**3,
        "existing_data": False, "existing_binary": False, "existing_unit": False,
    }
    result.update(overrides)
    return result


def command_missing(name, **overrides):
    h = host(**overrides)
    h["commands"][name] = False
    return h


class PreflightTests(unittest.TestCase):
    def test_clean_host_ready_both_roles(self):
        for role in ("node", "seed"):
            with self.subTest(role=role):
                result = pre.assess(role, host())
                self.assertEqual(result["assessment"], "READY")
                self.assertTrue(all(c["status"] == "PASS" for c in result["checks"]))

    def test_existing_operator_state_never_claimed_ready(self):
        for field in ("existing_data", "existing_binary", "existing_unit"):
            with self.subTest(field=field):
                self.assertEqual(pre.assess("node", host(**{field: True}))["assessment"], "REVIEW")

    def test_help_does_not_access_real_host(self):
        p = subprocess.run([sys.executable, str(ROOT / "node_kit_preflight.py"), "--help"],
                           capture_output=True, text=True, check=True)
        self.assertIn("--role", p.stdout)

    def test_insufficient_or_unknown_build_space(self):
        for free in (0, None, pre.MIN_FREE_BYTES - 1):
            with self.subTest(free=free):
                self.assertEqual(pre.assess("node", host(free_bytes=free))["assessment"], "REVIEW")
        self.assertEqual(pre.assess("node", host(free_bytes=pre.MIN_FREE_BYTES))["assessment"], "READY")

    def test_json_schema_and_exit_codes(self):
        for h, expected in ((host(), 0), (host(root=False), 1),
                            (command_missing("sudo"), 2)):
            with self.subTest(expected=expected), patch.object(pre, "inspect_host", return_value=h):
                output = io.StringIO()
                with redirect_stdout(output):
                    self.assertEqual(pre.main(["--role", "seed", "--json"]), expected)
                obj = json.loads(output.getvalue())
                self.assertEqual(obj["schema"], pre.SCHEMA)
                self.assertEqual(obj["role"], "seed")
                self.assertEqual(obj["scope"], "local-advisory-only-no-chain-or-network-verification")
                self.assertFalse(any("/var/lib" in c["detail"] for c in obj["checks"]))

    def test_linux_systemd_sudo_bash_are_hard_requirements(self):
        for h in (host(linux=False), host(systemd_running=False), command_missing("systemctl"),
                  command_missing("sudo"), command_missing("bash")):
            with self.subTest(h=h):
                self.assertEqual(pre.assess("node", h)["assessment"], "BLOCKED")

    def test_missing_cargo_and_package_manager_blocks(self):
        h = host()
        for name in ("cargo", "apt-get", "dnf"):
            h["commands"][name] = False
        self.assertEqual(pre.assess("node", h)["assessment"], "BLOCKED")

    def test_missing_git_when_cargo_preinstalled_blocks(self):
        self.assertEqual(pre.assess("node", command_missing("git"))["assessment"], "BLOCKED")

    def test_non_standard_arch_is_review_not_arbitrary_rejection(self):
        self.assertEqual(pre.assess("seed", host(arch="armv7l"))["assessment"], "REVIEW")

    def test_package_bootstrap_allows_review_not_false_ready(self):
        h = command_missing("git")
        h["commands"]["cargo"] = False
        self.assertEqual(pre.assess("seed", h)["assessment"], "REVIEW")

    def test_unprivileged_probe_is_not_installation_ready(self):
        self.assertEqual(pre.assess("node", host(root=False))["assessment"], "REVIEW")


if __name__ == "__main__":
    unittest.main()
