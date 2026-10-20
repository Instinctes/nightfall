#!/usr/bin/env python3
"""Offline Ed25519 challenge/response adversarial tests; no mainnet operations."""
import copy
import importlib.util
import json
import os
import pathlib
import subprocess
import tempfile
import unittest

PATH = pathlib.Path(__file__).with_name('operator-witness.py')
SPEC = importlib.util.spec_from_file_location('nightfall_operator_witness', PATH)
W = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(W)
CHALLENGE = '1a' * 32


def receipt(**updates):
    n = W.node
    s = dict(network='mainnet', genesis=n.GENESIS, protocol_version=8,
             wire_version=6, tip='a' * 64, tip_height=100, blocks=101,
             total_work='100000', minted=100000000, burned_fees=100,
             circulating=99999900, loading=False, supply_invariant_ok=True,
             live_peers=2, best_peer_height=100, stalled_on_fork=False,
             reorg_in_flight=False, mining=False, mining_idle='')
    s.update(updates)
    return n.seal(dict(schema='nightfall-node-evidence-v1',
                       scope='untrusted-local-rpc-claims-not-consensus-proof',
                       samples=[s, s], checks=['status', 'verify_supply'],
                       assessment='OBSERVED', warnings=[]))


class WitnessTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.keys = []
        for i in range(3):
            p = pathlib.Path(self.tmp.name) / f'key-{i}.pem'
            subprocess.run(['openssl', 'genpkey', '-algorithm', 'ED25519',
                            '-out', str(p)], check=True, capture_output=True)
            os.chmod(p, 0o600)
            self.keys.append(p)
        self.docs = [W.sign(receipt(), k, CHALLENGE) for k in self.keys]
        self.pins = {d['body']['operator'] for d in self.docs}

    def test_two_and_three_pinned_key_agreement(self):
        self.assertEqual(W.reconcile(self.docs[:2], self.pins, CHALLENGE)['result'],
                         'CONSISTENT_REPORTED_STATE')
        self.assertEqual(W.reconcile(self.docs, self.pins, CHALLENGE)['signed_distinct_keys'], 3)

    def test_tampering_and_key_substitution_rejected(self):
        altered = copy.deepcopy(self.docs[0])
        altered['body']['receipt']['payload']['samples'][1]['tip'] = 'b' * 64
        with self.assertRaises(W.WitnessError):
            W.verify(altered, self.pins, CHALLENGE)
        altered = copy.deepcopy(self.docs[0])
        altered['body']['operator'] = self.docs[1]['body']['operator']
        with self.assertRaises(W.WitnessError):
            W.verify(altered, self.pins, CHALLENGE)
        altered = copy.deepcopy(self.docs[0])
        altered['signature_b64'] = self.docs[1]['signature_b64']
        with self.assertRaises(W.WitnessError):
            W.verify(altered, self.pins, CHALLENGE)

    def test_key_pinning_and_no_double_counting(self):
        with self.assertRaises(W.WitnessError):
            W.verify(self.docs[0], {self.docs[1]['body']['operator']}, CHALLENGE)
        with self.assertRaises(W.WitnessError):
            W.reconcile([self.docs[0], self.docs[0]], self.pins, CHALLENGE)

    def test_fresh_challenge_blocks_replay(self):
        with self.assertRaises(W.WitnessError):
            W.verify(self.docs[0], self.pins, '2b' * 32)
        with self.assertRaises(W.WitnessError):
            W.sign(receipt(), self.keys[0], 'invalid')

    def test_divergence_never_claims_consensus(self):
        conflict = W.sign(receipt(tip='f' * 64), self.keys[1], CHALLENGE)
        self.assertEqual(W.reconcile([self.docs[0], conflict], self.pins, CHALLENGE)['result'],
                         'CONFLICTING_REPORTED_STATE')
        ahead = W.sign(receipt(tip_height=101, blocks=102), self.keys[1], CHALLENGE)
        self.assertEqual(W.reconcile([self.docs[0], ahead], self.pins, CHALLENGE)['result'],
                         'DIFFERENT_HEIGHTS_INCONCLUSIVE')

    def test_permissions_and_rejects_invalid_receipt(self):
        os.chmod(self.keys[0], 0o644)
        with self.assertRaises(W.WitnessError):
            W.sign(receipt(), self.keys[0], CHALLENGE)
        os.chmod(self.keys[0], 0o600)
        forged = receipt(circulating=5000)
        with self.assertRaises(W.WitnessError):
            W.sign(forged, self.keys[0], CHALLENGE)

    def test_trust_manifest_strict(self):
        trust = pathlib.Path(self.tmp.name) / 'trusted.json'
        trust.write_text(json.dumps({'schema': W.TRUST_SCHEMA,
                                     'fingerprints': sorted(self.pins)}))
        self.assertEqual(W.trust_file(trust), self.pins)
        trust.write_text(json.dumps({'schema': W.TRUST_SCHEMA,
                                     'fingerprints': [next(iter(self.pins))] * 2}))
        with self.assertRaises(W.WitnessError):
            W.trust_file(trust)


if __name__ == '__main__':
    unittest.main(verbosity=2)
