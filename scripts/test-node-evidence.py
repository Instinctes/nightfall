#!/usr/bin/env python3
"""Stdlib adversarial conformance tests (no node, wallet or internet needed)."""
import importlib.util
import json
import pathlib
import socket
import tempfile
import threading
import unittest

P = pathlib.Path(__file__).with_name("node-evidence.py")
S = importlib.util.spec_from_file_location("node_evidence", P)
kit = importlib.util.module_from_spec(S)
S.loader.exec_module(kit)


def state(**kwargs):
    fields = dict(network="mainnet", genesis=kit.GENESIS, protocol_version=8, wire_version=6,
                  tip="a" * 64, tip_height=100, blocks=101, total_work="101000",
                  minted=1_000_000_000, burned_fees=100, circulating=999_999_900,
                  loading=False, supply_invariant_ok=True, live_peers=3, best_peer_height=100,
                  stalled_on_fork=False, reorg_in_flight=False, mining=False, mining_idle="")
    fields.update(kwargs)
    return fields


class Tests(unittest.TestCase):
    def test_good(self):
        s = kit.check_status(state(extra_secret="NEVER_INCLUDE"))
        self.assertNotIn("extra_secret", s)
        kit.check_supply({"ok": True, "circulating_darks": s["circulating"]}, s)
        self.assertEqual([], kit.assess(s, s))
        kit.verify_receipt(kit.seal(dict(schema="nightfall-node-evidence-v1",
                   scope="untrusted-local-rpc-claims-not-consensus-proof", samples=[s, s],
                   checks=["status", "verify_supply"], assessment="OBSERVED", warnings=[])))

    def test_forged_money_and_genesis(self):
        for changes in [dict(genesis="0" * 64), dict(network="devnet"),
                        dict(circulating=999), dict(minted=kit.MAX_DARKS + 1),
                        dict(blocks=100), dict(tip_height=True),
                        dict(protocol_version=7), dict(supply_invariant_ok=1)]:
            with self.subTest(changes=changes), self.assertRaises(kit.EvidenceError):
                kit.check_status(state(**changes))

    def test_adversarial_parser(self):
        with self.assertRaises(kit.EvidenceError):
            kit.parse_json('{"id":1,"id":2}')
        with self.assertRaises(kit.EvidenceError):
            kit.parse_json('{"value":NaN}')
        with self.assertRaises(kit.EvidenceError):
            kit.check_supply({"ok": 1, "circulating_darks": 999_999_900}, state())

    def test_redaction_and_tamper(self):
        s = state()
        obj = kit.seal(dict(schema="nightfall-node-evidence-v1",
                            scope="untrusted-local-rpc-claims-not-consensus-proof",
                            samples=[s, s], checks=["status", "verify_supply"],
                            assessment="OBSERVED", warnings=[]))
        with tempfile.TemporaryDirectory() as tmp:
            a = pathlib.Path(tmp) / "a.json"
            b = pathlib.Path(tmp) / "b.json"
            a.write_text(json.dumps(obj))
            b.write_text(json.dumps(obj))
            self.assertEqual("SAME_REPORTED_TIP", kit.compare(a, b)["comparison"])
            obj["payload"]["samples"][1]["tip"] = "b" * 64
            b.write_text(json.dumps(obj))
            with self.assertRaises(kit.EvidenceError):
                kit.compare(a, b)
            obj["sha256"] = kit.digest(obj["payload"])
            b.write_text(json.dumps(obj))
            self.assertEqual("CONFLICTING_REPORTED_TIPS", kit.compare(a, b)["comparison"])

    def test_fork_signals(self):
        s = state()
        t = state(tip="b" * 64, live_peers=0, best_peer_height=300,
                  mining=True, mining_idle="", stalled_on_fork=True)
        w = kit.assess(s, t)
        self.assertIn("tip changed at same height", w)
        self.assertIn("no live peers", w)
        self.assertIn("mining enabled while behind with no explicit idle hold", w)
        self.assertIn("fork recovery active", w)

    def test_rpc_network_boundary(self):
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        port = listener.getsockname()[1]

        def serve():
            with listener:
                conn, _ = listener.accept()
                with conn:
                    data = conn.recv(8192)
                    req = json.loads(data)
                    body = {"result": state(), "error": None, "id": req["id"]}
                    conn.sendall(json.dumps(body).encode() + b"\n")

        thread = threading.Thread(target=serve)
        thread.start()
        try:
            self.assertEqual("mainnet", kit.rpc(port, "status", 47)["network"])
        finally:
            thread.join(timeout=5)


if __name__ == "__main__":
    unittest.main(verbosity=2)
