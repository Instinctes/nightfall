
import copy
import unittest

import payout_reference as p


class AccountingTests(unittest.TestCase):

    def setUp(self):

        self.sample = p.demo()


    def test_partial_oldest_share(self):

        result = p.calculate(
            self.sample
        )

        self.assertEqual(
            result["credited_work"],
            10
        )

        self.assertEqual(
            result["claimed_chain_work"],
            12
        )

        self.assertEqual(
            result["payouts"],
            [
                {
                    "miner": "alice",
                    "credited_work": 6,
                    "darks": 360_000_000
                },
                {
                    "miner": "bob",
                    "credited_work": 4,
                    "darks": 240_000_000
                }
            ]
        )


    def test_short_chain(self):

        self.sample["window_work"] = 100

        result = p.calculate(
            self.sample
        )

        self.assertEqual(
            result["credited_work"],
            12
        )

        self.assertEqual(
            result["payouts"][0]["darks"],
            400_000_000
        )

        self.assertEqual(
            result["payouts"][1]["darks"],
            200_000_000
        )


    def test_one_dark_tie_break(self):

        payout = p.allocate(
            {
                "zeta": 1,
                "alpha": 1,
                "beta": 1
            },
            1
        )

        self.assertEqual(
            payout,
            {
                "alpha": 1,
                "beta": 0,
                "zeta": 0
            }
        )


    def test_integer_conservation(self):

        for window in range(1, 30):

            self.sample["window_work"] = window

            for reward in (
                1,
                2,
                7,
                101,
                p.REWARD_DARKS
            ):

                result = p.calculate(
                    self.sample,
                    reward
                )

                total = sum(
                    x["darks"]
                    for x in result["payouts"]
                )

                self.assertEqual(
                    total,
                    reward
                )

                self.assertEqual(
                    result["credited_work"],
                    min(window, 12)
                )


    def test_replay_rejection(self):

        shares = self.sample["shares"]

        shares[2]["id"] = shares[0]["id"]

        with self.assertRaises(
            p.InputError
        ):

            p.calculate(
                self.sample
            )


    def test_divergent_parent(self):

        shares = self.sample["shares"]

        shares[1]["prev"] = p.ZERO_ID

        with self.assertRaises(
            p.InputError
        ):

            p.calculate(
                self.sample
            )


    def test_invalid_work(self):

        invalid_values = (
            -1,
            0,
            True,
            1 << 64,
            "99",
            2.5
        )

        for invalid in invalid_values:

            clone = copy.deepcopy(
                self.sample
            )

            clone["shares"][0]["work"] = invalid

            with self.assertRaises(
                p.InputError
            ):

                p.calculate(
                    clone
                )


    def test_bounded_reward(self):

        invalid_values = (
            0,
            -2,
            True,
            p.REWARD_DARKS + 1
        )

        for invalid in invalid_values:

            with self.assertRaises(
                p.InputError
            ):

                p.calculate(
                    self.sample,
                    invalid
                )


    def test_reject_extra_fields(self):

        clone = copy.deepcopy(
            self.sample
        )

        clone["rpc_password"] = "reject"

        with self.assertRaises(
            p.InputError
        ):

            p.calculate(
                clone
            )


    def test_empty_shares(self):

        clone = copy.deepcopy(
            self.sample
        )

        clone["shares"] = []

        with self.assertRaises(
            p.InputError
        ):

            p.calculate(
                clone
            )


    def test_duplicate_json_keys(self):

        with self.assertRaises(
            p.InputError
        ):

            p.parse_json(
                '{"shares":[],"shares":[]}'
            )


    def test_nonfinite_json(self):

        with self.assertRaises(
            p.InputError
        ):

            p.parse_json(
                '{"x":NaN}'
            )


    def test_deterministic_replay(self):

        a = p.calculate(
            self.sample
        )

        b = p.calculate(
            copy.deepcopy(self.sample)
        )

        self.assertEqual(
            a,
            b
        )

        self.assertEqual(
            p.digest(a),
            p.digest(b)
        )


    def test_fork_ancestry_rejection(self):

        fork = copy.deepcopy(
            self.sample
        )

        fork["shares"][2]["prev"] = (
            f"{99:064x}"
        )

        with self.assertRaises(
            p.InputError
        ):

            p.calculate(
                fork
            )


    def test_reject_sensitive_labels(self):

        labels = (
            "https://example.org",
            "nf1" + "a" * 70,
            "alice\nsecret",
            "../user"
        )

        for label in labels:

            clone = copy.deepcopy(
                self.sample
            )

            clone["shares"][0]["miner"] = label

            with self.assertRaises(
                p.InputError
            ):

                p.calculate(
                    clone
                )


if __name__ == "__main__":
    unittest.main()
