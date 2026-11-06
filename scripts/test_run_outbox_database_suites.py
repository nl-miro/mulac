import contextlib
import io
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import run_outbox_database_suites as suites


def completed(stdout="", returncode=0, stderr=""):
    return subprocess.CompletedProcess(["psql"], returncode, stdout=stdout, stderr=stderr)


def run_preflight(result=None, error=None):
    """Runs preflight against a mocked psql; returns (stdout, stderr, exit code)."""
    out, err = io.StringIO(), io.StringIO()
    code = None
    with patch.object(suites.subprocess, "run", return_value=result, side_effect=error), \
            contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            suites.preflight("postgres://example/outbox")
        except SystemExit as exit_:
            code = exit_.code
    return out.getvalue(), err.getvalue(), code


class ParsePreflightTests(unittest.TestCase):
    def test_plain_version(self):
        self.assertEqual(suites.parse_preflight("16.4,outbox\n"), ("16.4", "outbox"))

    def test_distribution_suffixed_version(self):
        output = '16.15 (Debian 16.15-1.pgdg13+1),outbox\n'
        self.assertEqual(suites.parse_preflight(output), ("16.15 (Debian 16.15-1.pgdg13+1)", "outbox"))

    def test_database_name_with_spaces_commas_and_quotes(self):
        output = '16.4,"my ""test"", db"\n'
        self.assertEqual(suites.parse_preflight(output), ("16.4", 'my "test", db'))

    def test_database_name_with_a_newline(self):
        self.assertEqual(suites.parse_preflight('16.4,"a\nb"\n'), ("16.4", "a\nb"))

    def test_malformed_output_is_rejected(self):
        for output in ["", "\n", "16.4\n", "16.4,outbox,extra\n", "16.4,outbox\n17.0,other\n", ",outbox\n", "16.4,\n"]:
            with self.subTest(output=output), self.assertRaises(ValueError):
                suites.parse_preflight(output)


class PreflightTests(unittest.TestCase):
    def test_reports_version_and_database(self):
        stdout, _, code = run_preflight(completed("16.15 (Debian 16.15-1.pgdg13+1),outbox\n"))
        self.assertIsNone(code)
        self.assertIn("PostgreSQL 16.15 (Debian 16.15-1.pgdg13+1), database outbox", stdout)

    def test_database_name_with_spaces_passes(self):
        stdout, _, code = run_preflight(completed('16.4,"outbox test"\n'))
        self.assertIsNone(code)
        self.assertIn("database outbox test", stdout)

    def test_failed_command_is_a_controlled_failure(self):
        _, stderr, code = run_preflight(completed(returncode=2, stderr="connection refused"))
        self.assertEqual(code, 1)
        self.assertIn("database preflight failed", stderr)
        self.assertIn("connection refused", stderr)

    def test_missing_psql_is_a_controlled_failure(self):
        _, stderr, code = run_preflight(error=FileNotFoundError("psql"))
        self.assertEqual(code, 1)
        self.assertIn("could not run psql", stderr)

    def test_malformed_output_is_a_controlled_failure(self):
        _, stderr, code = run_preflight(completed("not a record\n"))
        self.assertEqual(code, 1)
        self.assertIn("unexpected output", stderr)


class InventoryTests(unittest.TestCase):
    def tree(self, *files):
        root = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(root, ignore_errors=True))
        for relative in files:
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// no mention of the database url here\n")
        return root

    def test_a_suite_reaching_the_database_only_through_the_harness_is_still_caught(self):
        root = self.tree("libs/outbox/tests/new_suite.rs")
        self.assertEqual(len(suites.unclassified_suites(root, set())), 1)

    def test_inventoried_elsewhere_and_declared_database_free_files_are_classified(self):
        root = self.tree(
            "libs/outbox/tests/listed.rs",
            "libs/outbox/tests/prelude_smoke.rs",
            "test_apps/outbox_serde_matrix/tests/matrix.rs",
        )
        self.assertEqual(suites.unclassified_suites(root, {"listed"}), [])

    def test_a_test_app_suite_is_checked_too(self):
        root = self.tree("test_apps/outbox_other/tests/other.rs")
        self.assertEqual(len(suites.unclassified_suites(root, set())), 1)

    def test_the_shared_harness_directory_is_not_a_suite(self):
        root = self.tree("libs/outbox/tests/common/mod.rs")
        self.assertEqual(suites.unclassified_suites(root, set()), [])

    def test_missing_inventoried_and_declared_files_are_reported(self):
        root = self.tree("libs/outbox/tests/present.rs")
        problems = suites.missing_suites(root, {"present", "gone"})
        self.assertTrue(any("gone" in p for p in problems))
        self.assertFalse(any("present" in p and "does not exist" in p and "gone" not in p for p in problems))
        self.assertTrue(any("prelude_smoke.rs" in p for p in problems))

    def test_the_real_repository_is_fully_classified(self):
        inventoried = {sel[1] for _, _, sel, _, _ in suites.INVENTORY if sel[0] == "--test"}
        self.assertEqual(suites.unclassified_suites(suites.ROOT, inventoried), [])
        self.assertEqual(suites.missing_suites(suites.ROOT, inventoried), [])


if __name__ == "__main__":
    unittest.main()
