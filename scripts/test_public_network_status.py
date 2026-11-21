#!/usr/bin/env python3
"""Deterministic adversarial tests. Never contact a public node or website."""
import contextlib
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError

SCRIPT = Path(__file__).with_name(
    'check_public_network_status.py'
)

SPEC = importlib.util.spec_from_file_location(
    'nightfall_public_status',
    SCRIPT
)

checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def valid():
    return checker.fixture()


def endpoints(snapshot=None):
    snapshot = valid() if snapshot is None else snapshot
    amount = snapshot['circulating']

    return {
        '/api/supply/max': str(checker.CAP) + '\n',
        '/api/supply/total': (
            '%d.%08d' % divmod(amount, checker.UNIT)
        ),
        '/api/supply/circulating': (
            '%d.%08d' % divmod(amount, checker.UNIT)
        ),
    }


class FakeResponse:
    def __init__(
        self,
        body,
        url=checker.BASE + '/network.json'
    ):
        self.body = body
        self.url = url

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False

    def geturl(self):
        return self.url

    def read(self, size):
        return self.body[:size]


class PublicNetworkAdversarialTests(unittest.TestCase):

    def run_check(self, replies, clock=1100):
        seq = iter(replies)
        buf = io.StringIO()

        def reply(path):
            value = next(seq)

            if isinstance(value, Exception):
                raise value

            return value

        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / 'result.json'

            with patch.object(
                checker, 'get', side_effect=reply
            ), patch.object(
                checker.time, 'time', return_value=clock
            ), patch.object(
                checker.time, 'sleep'
            ) as sleeping, patch.object(
                sys, 'argv',
                ['check', '--report', str(report)]
            ), contextlib.redirect_stdout(buf):
                code = checker.main()

            payload = json.loads(
                report.read_text(encoding='utf-8')
            )

            self.assertEqual(
                payload,
                json.loads(buf.getvalue())
            )

            return (
                code,
                payload,
                sleeping.call_count
            )

    def replies(
        self,
        first=None,
        second=None,
        numbers=None
    ):
        first = valid() if first is None else first
        second = first if second is None else second
        numbers = (
            endpoints(first)
            if numbers is None
            else numbers
        )

        return [
            json.dumps(first),
            numbers['/api/supply/max'],
            numbers['/api/supply/total'],
            numbers['/api/supply/circulating'],
            json.dumps(second)
        ]

    def test_success_is_explicitly_not_a_cryptographic_proof(self):
        code, out, sleeps = self.run_check(
            self.replies()
        )

        self.assertEqual(
            (code, out['result'], sleeps),
            (0, 'CONSISTENT', 0)
        )

        self.assertIs(
            out['cryptographic_verification'],
            False
        )

        self.assertEqual(
            out['snapshot']['circulating'],
            12 * checker.UNIT - 1
        )

        self.assertEqual(
            out['tip_age_seconds'],
            100
        )

    def test_endpoint_discrepancy_is_reported_not_mislabelled_as_proof(self):
        nums = endpoints()
        nums['/api/supply/total'] = '1.00000000'

        code, out, sleeps = self.run_check(
            self.replies(numbers=nums)
        )

        self.assertEqual(
            (code, out['result'], sleeps),
            (1, 'MISMATCH', 0)
        )

        self.assertIn(
            'endpoints disagree',
            out['reason']
        )

        self.assertNotIn('snapshot', out)

    def test_moving_chain_tip_is_inconclusive_after_three_retries(self):
        other = dict(
            valid(),
            tip='b' * 64
        )

        code, out, sleeps = self.run_check(
            self.replies(second=other) * 3
        )

        self.assertEqual(
            (code, out['result'], sleeps),
            (2, 'INCONCLUSIVE', 2)
        )

        self.assertIn(
            'tip changed',
            out['reason']
        )

    def test_connection_outage_is_inconclusive_not_mismatch(self):
        code, out, sleeps = self.run_check(
            [OSError('offline')] * 3
        )

        self.assertEqual(
            (code, out['result'], sleeps),
            (2, 'INCONCLUSIVE', 2)
        )

    def test_stale_tip_is_inconclusive(self):
        code, out, sleeps = self.run_check(
            self.replies() * 3,
            clock=2000
        )

        self.assertEqual(
            (code, out['result'], sleeps),
            (2, 'INCONCLUSIVE', 2)
        )

        self.assertIn(
            'freshness',
            out['reason']
        )

    def test_loading_node_is_inconclusive(self):
        loading = json.dumps(
            dict(valid(), loading=True)
        )

        code, out, sleeps = self.run_check(
            [loading] * 3
        )

        self.assertEqual(
            (code, out['result'], sleeps),
            (2, 'INCONCLUSIVE', 2)
        )

    def test_same_tip_but_changed_claim_is_mismatch(self):
        changed = dict(
            valid(),
            burned_fees=2,
            circulating=12 * checker.UNIT - 2
        )

        code, out, sleeps = self.run_check(
            self.replies(second=changed)
        )

        self.assertEqual(
            (code, out['result'], sleeps),
            (1, 'MISMATCH', 0)
        )

        self.assertIn(
            'conflicting status',
            out['reason']
        )

    def test_wrong_cap_mismatch(self):
        nums = endpoints()
        nums['/api/supply/max'] = '90000001'

        code, out, sleeps = self.run_check(
            self.replies(numbers=nums)
        )

        self.assertEqual(
            (code, out['result'], sleeps),
            (1, 'MISMATCH', 0)
        )

    def test_malformed_supply_decimal_is_mismatch(self):
        nums = endpoints()
        nums['/api/supply/total'] = '1e15'

        code, out, sleeps = self.run_check(
            self.replies(numbers=nums)
        )

        self.assertEqual(
            (code, out['result'], sleeps),
            (1, 'MISMATCH', 0)
        )

    def test_bad_network_parameters_are_rejected(self):
        for key, value in (
            ('protocol_version', 9),
            ('wire_version', 7),
            ('protocol_version', True),
            ('genesis', '0' * 64),
            ('ticker', 'FAKE'),
            ('network', 'devnet'),
            ('blocks', -1),
            ('tip_time', -1)
        ):
            with self.subTest(
                key=key,
                value=value
            ), self.assertRaises(checker.Invalid):
                checker.status(
                    json.dumps(
                        dict(
                            valid(),
                            **{key: value}
                        )
                    )
                )

    def test_duplicate_json_keys_are_rejected(self):
        raw = (
            json.dumps(valid())[:-1]
            + ',"network":"mainnet"}'
        )

        with self.assertRaises(checker.Invalid):
            checker.status(raw)

    def test_full_emission_at_boundary(self):
        self.assertEqual(
            checker.minted_at(
                30 * checker.ERA
            ),
            8_999_999_925_000_000
        )

        self.assertEqual(
            checker.minted_at(10**9),
            checker.minted_at(
                30 * checker.ERA
            )
        )

        with self.assertRaises(checker.Invalid):
            checker.minted_at(True)

    def test_http_failure_translates_to_unavailable(self):
        error = HTTPError(
            checker.BASE + '/network.json',
            503,
            'service unavailable',
            {},
            None
        )

        with patch.object(
            checker,
            'urlopen',
            side_effect=error
        ):
            with self.assertRaises(checker.Unavailable):
                checker.get('/network.json')

    def test_redirect_outside_domain_is_rejected(self):
        response = FakeResponse(
            b'{}',
            'https://other.example/'
        )

        with patch.object(
            checker,
            'urlopen',
            return_value=response
        ):
            with self.assertRaises(checker.Unavailable):
                checker.get('/network.json')

    def test_oversized_response_is_rejected(self):
        response = FakeResponse(
            b'x' * 262145
        )

        with patch.object(
            checker,
            'urlopen',
            return_value=response
        ):
            with self.assertRaises(checker.Invalid):
                checker.get('/network.json')


if __name__ == '__main__':
    unittest.main()
