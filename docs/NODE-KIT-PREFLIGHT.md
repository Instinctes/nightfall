# Linux Node Kit preflight (advisory)

The roadmap calls for independent installation reports. This *optional* check
surfaces common host prerequisites and existing state **before** an operator
runs either official Linux/systemd installer. It never installs software.

Run on the **target Linux server** (not on an Android/Termux phone):

```sh
python3 scripts/node_kit_preflight.py --role node
python3 scripts/node_kit_preflight.py --role seed --json
```

- Exit **0 / READY**: the limited checks found no known blocker.
- Exit **1 / REVIEW**: an operator decision or toolchain bootstrap is needed.
- Exit **2 / BLOCKED**: a known installer prerequisite is missing.

The script checks Linux, a running systemd manager, indicative CPU architecture, `bash`, `systemctl`,
`sudo`, Git, the Rust bootstrap path, temporary build-space availability,
existing Node Kit data/binaries/service units, and whether the subsequent
installer will need root privileges. The 4 GiB temporary-space threshold is
**advisory**, not a resource guarantee. Architecture checks do not certify
Rust compilation. The probe produces no hostname, IP, wallet keys or file
contents and makes no network requests or changes to the host.

**Not a security audit, source signature, chain/genesis verification, network
reachability test, service health check, guarantee of a successful install, or
permission to overwrite existing state.** Existing operator data must be backed
up and inspected separately. A `READY` result is deliberately a narrow claim.

On Termux, the development regression tests work without Rust or root:

```sh
python3 -m unittest discover -s scripts -p 'test_node_kit_preflight.py' -v
```

These are fixture tests, not a claim to have tested installation on a VPS.
