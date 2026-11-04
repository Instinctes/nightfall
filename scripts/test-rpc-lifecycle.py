#!/usr/bin/env python3
"""Real >10-minute RPC/CLI regression. Disposable Devnet, literal loopback only."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
BIN = Path(os.environ.get("NIGHTFALL_LAB_BIN_DIR", ROOT / "target/debug"))


def port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def main():
    results = []
    children, sockets = [], []
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="nightfall-rpc-lifecycle-") as tmp:
        tmp = Path(tmp)
        rpc, p2p = port(), port()
        env = {**os.environ, "NIGHTFALL_PEERS_URL": ""}
        base = [str(BIN / "nightfalld"), "--network", "devnet", "--datadir", str(tmp / "node")]

        def connect():
            s = socket.create_connection(("127.0.0.1", rpc), timeout=5)
            s.settimeout(40)
            sockets.append(s)
            return s

        def request(s, method="status"):
            s.sendall((json.dumps({"method": method, "params": {}, "id": 1}) + "\n").encode())
            return json.loads(s.makefile("rb").readline())

        def passed(name):
            results.append(name)
            print("PASS", name, flush=True)

        try:
            subprocess.run(base + ["init"], check=True, stdout=subprocess.DEVNULL, env=env)
            with (tmp / "node.log").open("wb") as node_log, (tmp / "follow.log").open("wb") as follow_log:
                node = subprocess.Popen(base + ["run", "--listen", f"127.0.0.1:{p2p}", "--rpc-listen", f"127.0.0.1:{rpc}", "--proxy", "off"], env=env, stdout=node_log, stderr=subprocess.STDOUT)
                children.append(node)
                for _ in range(120):
                    try:
                        probe = connect()
                        assert request(probe)["error"] is None
                        probe.close()
                        break
                    except (OSError, AssertionError):
                        time.sleep(.25)
                else:
                    raise RuntimeError("disposable Devnet RPC did not start")
                wallet = [str(BIN / "nightfall-wallet"), "--network", "devnet", "--datadir", str(tmp / "wallet"), "--rpc", f"127.0.0.1:{rpc}"]
                subprocess.run(wallet + ["init"], check=True, stdout=subprocess.DEVNULL, env=env)
                follower = subprocess.Popen(wallet + ["follow"], env=env, stdout=follow_log, stderr=subprocess.STDOUT)
                children.append(follower)
                stream = connect()
                assert request(stream, "scan_subscribe")["error"] is None
                feed = stream.makefile("rb")
                subscription_start = time.monotonic()
                idle = connect()
                time.sleep(12)
                assert request(idle)["error"] is None
                idle.close()
                passed("ordinary RPC works after 12 seconds idle")
                slow = connect()
                slow.sendall(b'{"method":')
                time.sleep(12)
                try:
                    assert slow.recv(1) == b""
                except ConnectionResetError:
                    pass
                slow.close()
                passed("incomplete frame is disconnected after its 10 second budget")
                for _ in range(80):
                    assert follower.poll() is None, (tmp / "follow.log").read_text()
                    if "following tip" in (tmp / "follow.log").read_text():
                        break
                    time.sleep(.1)
                else:
                    raise AssertionError("real CLI did not enter follow mode")
                extra = []
                for _ in range(6):
                    s = connect()
                    assert request(s, "scan_subscribe")["error"] is None
                    extra.append(s)
                overflow = connect()
                assert "too many RPC subscriptions" in request(overflow, "scan_subscribe")["error"]
                overflow.close()
                probe = connect()
                assert request(probe)["error"] is None
                probe.close()
                passed("eight subscriptions bounded; ordinary status remains available")
                for s in extra:
                    s.close()
                heartbeats = 0
                while time.monotonic() - subscription_start <= 615:
                    line = feed.readline()
                    assert line, "subscription closed before long-run boundary"
                    answer = json.loads(line)
                    assert answer["error"] is None
                    heartbeats += bool(answer["result"].get("heartbeat"))
                    assert follower.poll() is None, "real wallet follow exited"
                    if heartbeats % 2 == 0:
                        print(f"progress: {time.monotonic() - subscription_start:.0f}s; follow alive", flush=True)
                assert heartbeats >= 20
                assert follower.poll() is None
                probe = connect()
                assert request(probe)["error"] is None
                probe.close()
                passed("real wallet follow and heartbeat stream survive beyond 615 seconds")
                print(json.dumps({"passed": results, "elapsed_seconds": round(time.monotonic() - started, 2), "network": "disposable-devnet-loopback", "binaries": str(BIN)}, indent=2), flush=True)
        finally:
            for s in sockets:
                s.close()
            for p in reversed(children):
                if p.poll() is None:
                    p.terminate()
                    try:
                        p.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        p.kill()
                        p.wait()


if __name__ == "__main__":
    main()
