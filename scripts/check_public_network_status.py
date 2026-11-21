#!/usr/bin/env python3
"""Public status consistency check; not a consensus or cryptographic verifier."""
import argparse
import json
import re
import sys
import time
import unittest
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

BASE = 'https://nightfallcoin.org'
GENESIS = '061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de'
UNIT, ERA, REWARD = 100_000_000, 7_500_000, 600_000_000
CAP = 90_000_000


class Invalid(ValueError):
    pass


class Unavailable(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise Invalid(message)


def uint(value, name):
    require(
        type(value) is int and value >= 0,
        name + ': invalid integer'
    )
    return value


def minted_at(blocks):
    uint(blocks, 'blocks')
    return sum(
        min(ERA, max(0, blocks - era * ERA))
        * (REWARD >> era)
        for era in range(REWARD.bit_length())
    )


def darks(text):
    text = text.strip()
    require(
        re.fullmatch(
            r'[0-9]{1,12}(?:\.[0-9]{1,8})?', text
        ) is not None,
        'invalid NIGHT decimal'
    )
    whole, _, fraction = text.partition('.')
    return int(whole) * UNIT + int(fraction.ljust(8, '0'))


def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, 'duplicate JSON key')
        result[key] = value
    return result


def status(raw):
    s = json.loads(raw, object_pairs_hook=pairs)

    require(type(s) is dict, 'status must be an object')

    require(
        s.get('genesis') == GENESIS
        and s.get('network') == 'mainnet'
        and s.get('ticker') == 'NIGHT',
        'wrong network identity'
    )

    require(
        type(s.get('protocol_version')) is int
        and s['protocol_version'] == 8
        and type(s.get('wire_version')) is int
        and s['wire_version'] == 6,
        'unsupported protocol/wire; review this checker'
    )

    if s.get('loading') is not False:
        raise Unavailable('node not ready')

    require(
        s.get('supply_invariant_ok') is True,
        'node did not report valid supply'
    )

    blocks = uint(s.get('blocks'), 'blocks')
    height = uint(s.get('tip_height'), 'tip_height')

    require(
        blocks == height + 1,
        'height/count mismatch'
    )

    tip = s.get('tip')

    require(
        type(tip) is str
        and re.fullmatch('[0-9a-f]{64}', tip) is not None,
        'invalid tip hash'
    )

    minted = uint(s.get('minted'), 'minted')
    burned = uint(s.get('burned_fees'), 'burned_fees')
    circulating = uint(s.get('circulating'), 'circulating')

    require(
        s.get('max_supply') == CAP
        and type(s.get('max_supply')) is int,
        'wrong maximum supply'
    )

    require(
        minted == minted_at(blocks) <= CAP * UNIT,
        'emission mismatch'
    )

    require(
        burned <= minted
        and circulating == minted - burned,
        'reported supply arithmetic mismatch'
    )

    timestamp = uint(s.get('tip_time'), 'tip_time')

    return {
        'height': height,
        'tip': tip,
        'minted': minted,
        'burned': burned,
        'circulating': circulating,
        'tip_time': timestamp
    }


def compare(a, b, endpoints):
    if (a['height'], a['tip']) != (b['height'], b['tip']):
        raise Unavailable('tip changed during sampling')

    require(
        a == b,
        'conflicting status for the same tip'
    )

    require(
        endpoints['max'] == CAP * UNIT,
        'maximum endpoint mismatch'
    )

    require(
        endpoints['total']
        == endpoints['circulating']
        == a['circulating'],
        'supply endpoints disagree on stable tip'
    )


def get(path):
    req = Request(
        BASE + path,
        headers={
            'User-Agent': 'Nightfall-public-evidence/1',
            'Cache-Control': 'no-cache',
            'Accept-Encoding': 'identity'
        }
    )

    try:
        with urlopen(req, timeout=20) as response:
            if not response.geturl().startswith(BASE + '/'):
                raise Unavailable('unexpected endpoint redirect')
            raw = response.read(262145)

    except HTTPError as exc:
        raise Unavailable(
            'HTTP ' + str(exc.code)
        ) from exc

    if len(raw) > 262144:
        raise Invalid('response exceeds size limit')

    return raw.decode('utf-8')


def fixture():
    return dict(
        genesis=GENESIS,
        network='mainnet',
        ticker='NIGHT',
        protocol_version=8,
        wire_version=6,
        loading=False,
        supply_invariant_ok=True,
        blocks=2,
        tip_height=1,
        tip='a' * 64,
        minted=12 * UNIT,
        burned_fees=1,
        circulating=12 * UNIT - 1,
        max_supply=CAP,
        tip_time=1000
    )


class Tests(unittest.TestCase):

    def test_emission(self):
        self.assertEqual(minted_at(0), 0)

        for era in range(30):
            boundary = era * ERA

            self.assertEqual(
                minted_at(boundary + 1)
                - minted_at(boundary),
                REWARD >> era
            )

        self.assertEqual(
            minted_at(30 * ERA),
            8_999_999_925_000_000
        )

        self.assertEqual(
            minted_at(31 * ERA),
            minted_at(30 * ERA)
        )

    def test_status(self):
        self.assertEqual(
            status(json.dumps(fixture()))['circulating'],
            12 * UNIT - 1
        )

        invalid = [
            ('minted', 1),
            ('burned_fees', -1),
            ('circulating', 0),
            ('blocks', True),
            ('genesis', 'wrong'),
            ('network', 'devnet'),
            ('supply_invariant_ok', 1),
            ('tip_height', 0)
        ]

        for key, value in invalid:
            with self.subTest(key=key), self.assertRaises(Invalid):
                status(
                    json.dumps(dict(fixture(), **{key: value}))
                )

        with self.assertRaises(Unavailable):
            status(json.dumps(dict(fixture(), loading=True)))

        with self.assertRaises(Invalid):
            json.loads(
                '{"a":1,"a":2}',
                object_pairs_hook=pairs
            )

    def test_decimal(self):
        self.assertEqual(darks('0.00000001'), 1)
        self.assertEqual(darks('90000000'), CAP * UNIT)

        for bad in (
            'NaN', '-1', '1e3',
            '0.000000001', 'inf'
        ):
            with self.assertRaises(Invalid):
                darks(bad)

    def test_compare(self):
        a = status(json.dumps(fixture()))

        good = dict(
            max=CAP * UNIT,
            total=a['circulating'],
            circulating=a['circulating']
        )

        compare(a, a, good)

        with self.assertRaises(Unavailable):
            compare(a, dict(a, tip='b' * 64), good)

        with self.assertRaises(Invalid):
            compare(a, dict(a, burned=2), good)

        with self.assertRaises(Invalid):
            compare(a, a, dict(good, total=0))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()

    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(
            Tests
        )
        return (
            0 if unittest.TextTestRunner(
                verbosity=2
            ).run(suite).wasSuccessful() else 1
        )

    result = {
        'scope': 'public-endpoint-consistency-only',
        'cryptographic_verification': False,
        'observed_at': int(time.time()),
        'result': 'INCONCLUSIVE'
    }

    for attempt in range(3):
        try:
            a = status(get('/network.json'))

            nums = {
                k: darks(get('/api/supply/' + k))
                for k in ('max', 'total', 'circulating')
            }

            b = status(get('/network.json'))

            compare(a, b, nums)

            age = int(time.time()) - b['tip_time']

            if not -300 <= age <= 600:
                raise Unavailable(
                    'tip timestamp outside freshness window'
                )

            result.update(
                result='CONSISTENT',
                snapshot=b,
                tip_age_seconds=age
            )

            break

        except Invalid as exc:
            result.update(
                result='MISMATCH',
                reason=str(exc)
            )
            break

        except (Unavailable, OSError, ValueError) as exc:
            result['reason'] = str(exc)

            if attempt < 2:
                time.sleep(2)

    if result['result'] == 'CONSISTENT':
        result.pop('reason', None)

    output = json.dumps(
        result, indent=2, sort_keys=True
    ) + '\n'

    if args.report:
        args.report.write_text(
            output, encoding='utf-8'
        )

    print(output, end='')

    return {
        'CONSISTENT': 0,
        'MISMATCH': 1,
        'INCONCLUSIVE': 2
    }[result['result']]


if __name__ == '__main__':
    sys.exit(main())
