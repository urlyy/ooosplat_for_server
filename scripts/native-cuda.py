#!/usr/bin/env python3
"""Validate the installed toolkit for a native build; release locks stay unchanged."""
import argparse
import json
from pathlib import Path
import re
import subprocess


def parse_version(output):
    match = re.search(r'release (\d+\.\d+), V([\d.]+)', output)
    if not match or match[1] not in {'12.8', '13.0', '13.2'}:
        raise ValueError('Native builds require CUDA Toolkit 12.8, 13.0 or 13.2 (nvcc), not only a driver')
    return dict(release=match[1], compilerVersion=match[2])


def inspect(root, build=None):
    nvcc = (root / 'bin/nvcc').resolve(strict=True)
    info = parse_version(subprocess.check_output([str(nvcc), '--version'], text=True))
    info.update(root=str(root.resolve()), compiler=str(nvcc))
    if build:
        cache = (build / 'CMakeCache.txt').read_text()
        match = re.search(r'^CMAKE_CUDA_COMPILER:[^=]+=(.+)$', cache, re.M)
        if not match or Path(match[1]).resolve() != nvcc:
            raise ValueError('CMake selected a different CUDA compiler')
        architectures = re.search(r'^CMAKE_CUDA_ARCHITECTURES:[^=]+=(.+)$', cache, re.M)
        if not architectures:
            raise ValueError('CMake did not record CUDA architectures')
        info['architectures'] = architectures[1]
    return info


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--build', type=Path)
    args = parser.parse_args()
    print(json.dumps(inspect(args.root, args.build), indent=2))
