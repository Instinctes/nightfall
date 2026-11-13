#!/usr/bin/env python3
"""Real integration of collection and upgrade gate."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import nightwatch as nw
from test_nightwatch import fixture


class IntegrationTests(unittest.TestCase):

    def test_automatic_upgrade_evidence(self):

        with tempfile.TemporaryDirectory() as temp:

            directory = Path(temp)

            claim = nw.sanitize(fixture())

            with (
                patch.object(nw, "OUT", directory),
                patch.object(
                    nw,
                    "fetch",
                    return_value=claim,
                ),
                patch.object(
                    nw,
                    "load_history",
                    return_value=[],
                ),
            ):

                nw.sample()

            history = directory / "history.json"
            upgrade = directory / "upgrade-gate.json"
            report = directory / "report.json"

            self.assertTrue(history.is_file())
            self.assertTrue(upgrade.is_file())
            self.assertTrue(report.is_file())

            result = json.loads(
                upgrade.read_text()
            )

            self.assertEqual(
                result["latest_height"],
                claim["tip_height"],
            )

            self.assertEqual(
                result["reported_compatible_labels"],
                49,
            )

            self.assertEqual(
                result["reported_legacy_labels"],
                0,
            )


if __name__ == "__main__":
    unittest.main(verbosity=2)
