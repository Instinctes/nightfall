#!/usr/bin/env python3
"""10+ minute integration regression for nightfall-wallet follow (PR #51).

Run real node and wallet binaries on isolated devnet data. Before PR #51's
stream-lifetime fix, follow returned silently when RPC closed its subscription
after 600 seconds. This is a slow, intentionally separate CI smoke test.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time


def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def status(port):
    with socket.create_connection(("127.0.0.1", port), timeout=3) as s:
        s.settimeout(3)
        s.sendall(b'{"method":"status","params":{},"id":1}\n')
        with s.makefile("rb") as reader:
            line = reader.readline(1048576)
    response = json.loads(line)
    if response.get("error"):
        raise RuntimeError("RPC error: " + str(response["error"]))
    result = response.get("result")
    if not isinstance(result, dict) or result.get("network") != "devnet":
        raise RuntimeError("RPC is not the expected devnet")
    return result


def stop(proc):
    if proc is None or proc.poll() is not None:
        return
    if os.name == "posix":
        os.killpg(proc.pid, signal.SIGTERM)
    else:
        proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        if os.name == "posix":
            os.killpg(proc.pid, signal.SIGKILL)
        else:
            proc.kill()
        proc.wait(timeout=5)


def tail(path):
    if not path.exists():
        return "<no log>"
    return path.read_text(encoding="utf-8", errors="replace")[-3000:]


def running(proc, name, log):
    code = proc.poll()
    if code is not None:
        raise RuntimeError(f"{name} exited rc={code}\n{tail(log)}")


def wait_node(proc, logfile, port, deadline):
    while True:
        running(proc, "nightfalld", logfile)
        try:
            status(port)
            return
        except (OSError, RuntimeError, ValueError):
            if time.monotonic() >= deadline:
                raise RuntimeError("node RPC startup timeout\n" + tail(logfile))
            time.sleep(0.5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", type=Path, required=True)
    parser.add_argument("--wallet", type=Path, required=True)
    parser.add_argument("--seconds", type=int, default=630)
    parser.add_argument("--startup-seconds", type=int, default=90)
    a = parser.parse_args()
    if a.seconds <= 600:
        parser.error("duration must exceed the old 600-second stream lifetime")
    if a.startup_seconds < 1:
        parser.error("startup timeout must be positive")
    node = a.node.resolve(strict=True)
    wallet = a.wallet.resolve(strict=True)
    if not os.access(node, os.X_OK) or not os.access(wallet, os.X_OK):
        parser.error("node and wallet must be executable")

    with tempfile.TemporaryDirectory(prefix="nf-follow-devnet-") as temporary:
        base = Path(temporary)
        node_dir, wallet_dir = base / "node", base / "wallet"
        node_dir.mkdir()
        wallet_dir.mkdir()
        node_log, wallet_log = base / "node.log", base / "wallet.log"
        rpc_port = free_port()
        env = dict(os.environ)
        env["NIGHTFALL_PEERS_URL"] = "off"
        env.pop("NF_ALLOW_PUBLIC_RPC", None)

        node_args = [str(node), "--network", "devnet", "--datadir", str(node_dir)]
        subprocess.run(node_args + ["init"], check=True, stdout=subprocess.DEVNULL,
                       stderr=subprocess.PIPE, timeout=a.startup_seconds, env=env)

        node_proc = wallet_proc = None
        try:
            with node_log.open("w", encoding="utf-8") as nout, \
                 wallet_log.open("w", encoding="utf-8") as wout:
                node_proc = subprocess.Popen(node_args + [
                    "run", "--listen", "127.0.0.1:0",
                    "--rpc-listen", f"127.0.0.1:{rpc_port}",
                    "--proxy", "off"], stdin=subprocess.DEVNULL,
                    stdout=nout, stderr=subprocess.STDOUT,
                    start_new_session=True, env=env)
                wait_node(node_proc, node_log, rpc_port,
                          time.monotonic() + a.startup_seconds)

                wallet_args = [str(wallet), "--network", "devnet",
                               "--datadir", str(wallet_dir),
                               "--rpc", f"127.0.0.1:{rpc_port}"]
                subprocess.run(wallet_args + ["init"], check=True,
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                               timeout=a.startup_seconds, env=env)
                wallet_proc = subprocess.Popen(wallet_args + ["follow"],
                                               stdin=subprocess.DEVNULL, stdout=wout,
                                               stderr=subprocess.STDOUT,
                                               start_new_session=True, env=env)
                deadline = time.monotonic() + a.startup_seconds
                while "following tip" not in tail(wallet_log):
                    running(node_proc, "nightfalld", node_log)
                    running(wallet_proc, "nightfall-wallet follow", wallet_log)
                    if time.monotonic() >= deadline:
                        raise RuntimeError("follow did not begin\n" + tail(wallet_log))
                    time.sleep(0.5)

                begin = time.monotonic()
                print("Client is following; 630-second survival test started", flush=True)
                next_status = begin
                while time.monotonic() - begin < a.seconds:
                    running(node_proc, "nightfalld", node_log)
                    running(wallet_proc, "nightfall-wallet follow", wallet_log)
                    if time.monotonic() >= next_status:
                        status(rpc_port)
                        next_status = time.monotonic() + 30
                    time.sleep(1)
                running(node_proc, "nightfalld", node_log)
                running(wallet_proc, "nightfall-wallet follow", wallet_log)
                status(rpc_port)
                print(f"PASS: real follow survived {time.monotonic()-begin:.1f}s",
                      flush=True)
        except Exception:
            print("NODE LOG:\n" + tail(node_log), file=sys.stderr)
            print("WALLET LOG:\n" + tail(wallet_log), file=sys.stderr)
            raise
        finally:
            stop(wallet_proc)
            stop(node_proc)


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.SubprocessError, ValueError, RuntimeError) as e:
        print("FAIL:", e, file=sys.stderr)
        sys.exit(1)
