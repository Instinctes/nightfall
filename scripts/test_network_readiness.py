"""
Adversarial offline tests.

No network access.
No wallet access.
No credentials.
"""

import importlib.util
import pathlib
import unittest


spec = importlib.util.spec_from_file_location(

    "network_readiness",

    pathlib.Path(__file__).with_name(
        "network_readiness.py"
    )
)

mod = importlib.util.module_from_spec(
    spec
)

spec.loader.exec_module(
    mod
)

NOW = 1_794_163_324


def valid():

    blocks = 482_210

    minted = mod.emission(
        blocks
    )

    return {

        "blocks": blocks,

        "tip_height": blocks - 1,

        "tip": "a" * 64,

        "minted": minted,

        "burned_fees": 2_393_800_000,

        "circulating": (
            minted - 2_393_800_000
        ),

        "max_supply": 90_000_000,

        "difficulty": 253_046,

        "peers": 41,

        "loading": False,

        "tip_time": NOW,

        "genesis": mod.GENESIS,

        "network": "mainnet",

        "protocol_version": 8,

        "wire_version": 6,

        "supply_invariant_ok": True,

        "total_work": "100657641840",

        "peer_versions": {
            "nightfalld/1.1.2": 30
        }
    }


class ReadinessTests(unittest.TestCase):

    def check(
        self,
        payload,
        expected="FAIL",
        previous=None
    ):

        report = mod.evaluate(

            payload,

            previous=previous,

            now=NOW
        )

        self.assertEqual(

            report["verdict"],

            expected,

            report
        )

        return report

    def test_valid(self):

        self.check(
            valid(),
            "WARN"
        )

    def test_exact_current_emission(self):

        self.assertEqual(

            mod.emission(482210),

            289326000000000
        )

    def test_era_boundaries(self):

        b = mod.ERA_LENGTH

        self.assertEqual(

            mod.emission(b),

            b * 600000000
        )

        self.assertEqual(

            mod.emission(b + 1),

            b * 600000000 + 300000000
        )

    def test_tail_no_float(self):

        self.assertEqual(

            mod.emission(225_000_000),

            8_999_999_925_000_000
        )

        self.assertEqual(

            mod.emission(900_000_000),

            mod.emission(225_000_000)
        )

    def test_wrong_minted(self):

        x = valid()

        x["minted"] += 1

        self.check(x)

    def test_hidden_inflation_with_matching_circulation(self):

        x = valid()

        x["minted"] += 123

        x["circulating"] += 123

        self.check(x)

    def test_false_invariant(self):

        x = valid()

        x["supply_invariant_ok"] = False

        self.check(x)

    def test_wrong_genesis(self):

        x = valid()

        x["genesis"] = "f" * 64

        self.check(x)

    def test_negative_burn(self):

        x = valid()

        x["burned_fees"] = -1

        self.check(x)

    def test_bool_currency_is_not_integer(self):

        x = valid()

        x["minted"] = True

        self.check(x)

    def test_float_currency(self):

        x = valid()

        x["minted"] = float(
            x["minted"]
        )

        self.check(x)

    def test_height_count_disagreement(self):

        x = valid()

        x["tip_height"] += 1

        self.check(x)

    def test_invalid_work(self):

        x = valid()

        x["total_work"] = "1e30"

        self.check(x)

    def test_version_legacy_flag(self):

        x = valid()

        x["peer_versions"][
            "nightfalld/1.0.11"
        ] = 3

        r = self.check(
            x,
            "WARN"
        )

        self.assertEqual(

            r["facts"][
                "old_version_label_observations"
            ],

            3
        )

    def test_unknown_peer_version(self):

        x = valid()

        x["peer_versions"] = {
            "future/experimental": 2
        }

        self.check(
            x,
            "WARN"
        )

    def test_previous_sample_divergence(self):

        x = valid()

        previous = {

            "tip_height": (
                x["tip_height"] + 1
            ),

            "total_work": "999999999999"
        }

        self.check(
            x,
            "WARN",
            previous
        )

    def test_stale_tip(self):

        x = valid()

        x["tip_time"] -= 3601

        self.check(
            x,
            "WARN"
        )

    def test_future_tip(self):

        x = valid()

        x["tip_time"] += 900

        self.check(x)

    def test_wrong_protocol(self):

        x = valid()

        x["protocol_version"] = 9

        self.check(x)

    def test_missing_data(self):

        self.check({})

    def test_no_wallet_details_in_report(self):

        x = valid()

        x["private_key"] = "NEVER LEAK"

        x["address"] = "NEVER LEAK"

        r = mod.evaluate(
            x,
            now=NOW
        )

        self.assertNotIn(
            "NEVER LEAK",
            repr(r)
        )

    def test_peers_zero_is_warning(self):

        x = valid()

        x["peers"] = 0

        self.check(
            x,
            "WARN"
        )

    def test_post_activation(self):

        x = valid()

        x["blocks"] = 500_002

        x["tip_height"] = 500_001

        x["minted"] = mod.emission(
            x["blocks"]
        )

        x["circulating"] = (
            x["minted"] - x["burned_fees"]
        )

        self.check(
            x,
            "PASS"
        )


if __name__ == "__main__":

    unittest.main()
