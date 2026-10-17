import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('native_cuda', Path(__file__).resolve().parents[1] / 'scripts/native-cuda.py')
cuda = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cuda)


class NativeCudaTest(unittest.TestCase):
    def test_supported_toolkits(self):
        for version in ('12.8', '13.0', '13.2'):
            parsed = cuda.parse_version(f'Cuda compilation tools, release {version}, V{version}.88')
            self.assertEqual(parsed['release'], version)

    def test_driver_output_is_not_a_toolkit(self):
        for output in ('CUDA Version: 13.2', 'Cuda compilation tools, release 11.8, V11.8.89', ''):
            with self.assertRaises(ValueError):
                cuda.parse_version(output)
