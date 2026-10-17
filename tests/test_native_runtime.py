import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts/native-runtime.py'
spec = importlib.util.spec_from_file_location('native_runtime', SCRIPT)
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class NativeRuntimeTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.addCleanup(self.temp.cleanup)

    def metadata(self):
        return dict(schemaVersion=1, kind='ubuntu22.04-native-source',
                    files={str(p.relative_to(self.root)): native.digest(p) for p in native.files(self.root)})

    def test_inventory_rejects_modified_and_extra_files(self):
        binary = self.root / 'engine'
        binary.write_bytes(b'native binary')
        metadata = self.metadata()
        native.verify_inventory(self.root, metadata)
        binary.write_bytes(b'changed')
        with self.assertRaises(ValueError):
            native.verify_inventory(self.root, metadata)
        binary.write_bytes(b'native binary')
        (self.root / 'extra-library.so').write_bytes(b'extra')
        with self.assertRaises(ValueError):
            native.verify_inventory(self.root, metadata)

    def test_reject_path_escape_and_external_symlink(self):
        with self.assertRaises(ValueError):
            native.contained(self.root, '../elsewhere')
        (self.root / 'escape').symlink_to(self.root.parent, target_is_directory=True)
        with self.assertRaises(ValueError):
            native.contained(self.root, 'escape/outside')

    def test_vocabulary_checks_size_and_hash(self):
        vocab = self.root / 'vocabulary'
        vocab.write_bytes(b'123')
        expected = dict(bytes=3, sha256=native.digest(vocab))
        native.verify_vocabulary(vocab, expected)
        vocab.write_bytes(b'456')
        with self.assertRaises(ValueError):
            native.verify_vocabulary(vocab, expected)

    def test_activation_preserves_previous_release(self):
        engines = self.root / 'engines'
        engines.mkdir()
        first = engines / '.native-release-first'
        second = engines / '.native-release-second'
        for release in [first, second]:
            release.mkdir()
            (release / 'NATIVE-BUILD.json').write_text('{}')
        with patch.object(native, 'ROOT', self.root):
            native.activate(first)
            self.assertEqual((engines / 'native-linux').resolve(), first)
            native.activate(second)
            self.assertEqual((engines / 'native-linux').resolve(), second)
            self.assertTrue(first.is_dir())
            (engines / 'native-linux').unlink()
            (engines / 'native-linux').mkdir()
            with self.assertRaises(ValueError):
                native.activate(first)


if __name__ == '__main__':
    unittest.main()
