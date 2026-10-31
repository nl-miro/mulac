import hashlib
import json
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import patch

from pack_and_publish import PACKAGES, ROOT, add_archive, index_path


class PublishingTests(unittest.TestCase):
    def test_index_paths(self):
        for name, expected in [("a", "1/a"), ("ab", "2/ab"), ("ABC", "3/a/abc"),
                               ("mulac-kernel", "mu/la/mulac-kernel")]:
            self.assertEqual(index_path(name), expected)

    def test_order_covers_workspace_and_places_dependencies_first(self):
        workspace = tomllib.loads((ROOT / "libs/Cargo.toml").read_text())["workspace"]
        self.assertEqual(set(PACKAGES), {f"libs/{m}" for m in workspace["members"]} | {"kernel"})
        positions = {(ROOT / directory).resolve(): i for i, directory in enumerate(PACKAGES)}
        for directory in PACKAGES:
            manifest = tomllib.loads((ROOT / directory / "Cargo.toml").read_text())
            for name, dep in manifest.get("dependencies", {}).items():
                base = ROOT / directory
                if isinstance(dep, dict) and dep.get("workspace"):
                    dep = workspace["dependencies"][name]
                    base = ROOT / "libs"
                if isinstance(dep, dict) and "path" in dep:
                    dependency = (base / dep["path"]).resolve()
                    self.assertLess(positions[dependency], positions[(ROOT / directory).resolve()])

    def test_identical_retry_does_not_call_margo_and_conflict_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "test.crate"
            archive.write_bytes(b"archive")
            index = root / index_path("test")
            index.parent.mkdir(parents=True)
            index.write_text(json.dumps({"vers": "0.4.0", "cksum": hashlib.sha256(b"archive").hexdigest()}))
            stored = root / "crates" / index_path("test") / "0.4.0.crate"
            stored.parent.mkdir(parents=True)
            stored.write_bytes(b"archive")
            with patch("pack_and_publish.subprocess.run") as run:
                add_archive(root, archive, "test", "0.4.0", "margo")
                run.assert_not_called()
                archive.write_bytes(b"different")
                with self.assertRaisesRegex(RuntimeError, "different bytes"):
                    add_archive(root, archive, "test", "0.4.0", "margo")
                run.assert_not_called()

    def test_successful_cli_exit_without_registry_files_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "test.crate"
            archive.write_bytes(b"archive")
            with patch("pack_and_publish.subprocess.run"):
                with self.assertRaisesRegex(RuntimeError, "verification failed"):
                    add_archive(root, archive, "test", "0.4.0", "margo")


if __name__ == "__main__":
    unittest.main()
