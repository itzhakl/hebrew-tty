import copy
import importlib.util
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "terminal_proxy_probe", ROOT / "tools" / "terminal_proxy_probe.py"
)
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


class ClassifierTests(unittest.TestCase):
    def test_normal_wrapped_marker_rows_are_post_bidi(self):
        result = PROBE.classify([
            "HTP_A הדגבא HTP_B 0123456789 abcDEF HTP_C",
            "HTP_D יטחזו",
        ], 48)

        self.assertEqual(result["order"], "visual")
        self.assertEqual(result["wrapping"], "post_bidi")

    def test_reversed_wrapped_marker_rows_are_pre_bidi(self):
        result = PROBE.classify([
            "HTP_C יטחזו HTP_D",
            "HTP_A הדגבא HTP_B 0123456789 abcDEF",
        ], 48)

        self.assertEqual(result["order"], "visual")
        self.assertEqual(result["wrapping"], "pre_bidi")

    def test_ambiguous_wrapped_marker_rows_block(self):
        with self.assertRaisesRegex(SystemExit, "wrapped marker order"):
            PROBE.classify([
                "HTP_A הדגבא HTP_C",
                "HTP_B 0123456789 abcDEF HTP_D יטחזו",
            ], 48)


class SchemaContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        fixture_path = (
            ROOT / "test" / "fixtures" / "terminal-proxy" / "measurements"
            / "claude-direct-48.json"
        )
        cls.document = json.loads(fixture_path.read_text())

    def test_schema_version_two_document_matches_validator(self):
        document = copy.deepcopy(self.document)
        document["classification"] = PROBE.classify(
            document["observed"]["probe_fragments"], document["terminal"]["columns"]
        )

        self.assertEqual(PROBE.validate_document("synthetic", document), [])

    def test_validator_rejects_unknown_fields_and_observed_source(self):
        document = copy.deepcopy(self.document)
        document["unexpected"] = True
        document["observed"]["source"] = "paint_fragments"

        errors = PROBE.validate_document("fixture", document)

        self.assertTrue(any("top-level schema fields differ" in error for error in errors))
        self.assertTrue(any("reconstructed from terminal rows" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
