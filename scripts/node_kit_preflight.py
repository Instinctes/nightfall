#!/usr/bin/env python3
"""Read-only readiness checks for Nightfall's Linux systemd Node Kit installers.

No network calls, privileged operations, wallet reads, or filesystem writes.
The assessment is advisory; it is NOT an installation/security attestation.
"""
import argparse
import json
import os
import platform
import shutil
import sys
from pathlib import Path

SCHEMA = "nightfall-node-kit-preflight-v1"
MIN_FREE_BYTES = 4 * 1024**3  # Advisory build-workspace heuristic, not a guarantee.


def inspect_host(role):
    """Collect only local capability booleans. Never open operator data files."""
    commands = {name: shutil.which(name) is not None for name in
                ("bash", "systemctl", "sudo", "git", "cargo", "apt-get", "dnf")}
    try:
        free = shutil.disk_usage(os.environ.get("TMPDIR", "/tmp")).free
    except OSError:
        free = None
    service = "nightfall-node" if role == "node" else "nightfall-seed"
    return {
        "linux": platform.system() == "Linux",
        "systemd_running": Path("/run/systemd/system").is_dir(),
        "arch": platform.machine().lower(),
        "root": os.geteuid() == 0,
        "commands": commands,
        "free_bytes": free,
        "existing_data": (Path("/var/lib/nightfall").exists() or Path("/var/lib/nightfall").is_symlink()),
        "existing_binary": (Path("/usr/local/bin/nightfalld").exists() or Path("/usr/local/bin/nightfalld").is_symlink()),
        "existing_unit": (Path("/etc/systemd/system", service + ".service").exists() or
                          Path("/etc/systemd/system", service + ".service").is_symlink()),
    }


def assess(role, host):
    """Pure decision engine so every risky branch is unit-testable offline."""
    checks = []

    def check(code, status, detail):
        checks.append({"code": code, "status": status, "detail": detail})

    check("LINUX", "PASS" if host["linux"] else "BLOCK",
          "Official installers require Linux with systemd.")
    check("SYSTEMD_RUNTIME", "PASS" if host["systemd_running"] else "BLOCK",
          "A running systemd manager is required, not just a systemctl binary.")
    check("ARCH", "PASS" if host["arch"] in ("x86_64", "aarch64") else "REVIEW",
          "x86_64/aarch64 are expected; verify Rust target support for other CPUs.")
    c = host["commands"]
    for name in ("bash", "systemctl", "sudo"):
        check(name.upper(), "PASS" if c[name] else "BLOCK",
              name + " is called by the existing installers and must be available.")
    if c["git"]:
        check("GIT", "PASS", "Git is available.")
    else:
        status = "REVIEW" if not c["cargo"] and (c["apt-get"] or c["dnf"]) else "BLOCK"
        check("GIT", status, "Git is needed for source checkout; dependency bootstrap may supply it.")
    if c["cargo"]:
        check("RUST", "PASS", "Cargo exists; version and build success are not tested.")
    elif c["apt-get"] or c["dnf"]:
        check("RUST", "REVIEW", "Installer may bootstrap a toolchain; network access is required.")
    else:
        check("RUST", "BLOCK", "No Cargo or supported package manager detected.")
    if host["free_bytes"] is None:
        check("WORKSPACE", "REVIEW", "Could not measure free temporary build space.")
    else:
        check("WORKSPACE", "PASS" if host["free_bytes"] >= MIN_FREE_BYTES else "REVIEW",
              "4 GiB free temporary space is an advisory threshold, not a build guarantee.")
    prior = any(host[k] for k in ("existing_data", "existing_binary", "existing_unit"))
    check("EXISTING_INSTALL", "REVIEW" if prior else "PASS",
          "Existing data/binary/unit requires backup and upgrade planning; no files were opened.")
    check("ROOT_INSTALL", "PASS" if host["root"] else "REVIEW",
          "Check can run unprivileged; actual official installer must run as root.")
    overall = ("BLOCKED" if any(x["status"] == "BLOCK" for x in checks)
               else "REVIEW" if any(x["status"] == "REVIEW" for x in checks)
               else "READY")
    return {"schema": SCHEMA, "role": role, "assessment": overall, "checks": checks,
            "scope": "local-advisory-only-no-chain-or-network-verification"}


def main(argv=None):
    parser = argparse.ArgumentParser(description="Offline, read-only Linux Node Kit readiness probe")
    parser.add_argument("--role", choices=("node", "seed"), required=True)
    parser.add_argument("--json", action="store_true", help="Machine-readable non-sensitive summary")
    args = parser.parse_args(argv)
    result = assess(args.role, inspect_host(args.role))
    if args.json:
        print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    else:
        print("NIGHTFALL NODE KIT PREFLIGHT — " + result["assessment"])
        for item in result["checks"]:
            print("{:<7} {:<18} {}".format(item["status"], item["code"], item["detail"]))
        print("Advisory only; no network, chain, wallet, or installer verification.")
    return {"READY": 0, "REVIEW": 1, "BLOCKED": 2}[result["assessment"]]


if __name__ == "__main__":
    sys.exit(main())
