#!/usr/bin/env python3
"""Install data, record and verify locally built Ubuntu 22.04 engines."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
VOCAB_URL = ('https://github.com/colmap/colmap/releases/download/3.11.1/'
             'vocab_tree_faiss_flickr100K_words256K.bin')


def load(path):
    return json.loads(path.read_text())


def digest(path):
    sha = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            sha.update(chunk)
    return sha.hexdigest()


def contained(root, relative):
    path = root / relative
    if Path(relative).is_absolute() or '..' in Path(relative).parts or not path.resolve().is_relative_to(root.resolve()):
        raise ValueError(f'Runtime path escapes root: {relative}')
    return path


def verify_vocabulary(path, expected):
    if path.stat().st_size != expected['bytes'] or digest(path) != expected['sha256']:
        raise ValueError('Offline vocabulary size/SHA-256 mismatch')


def vocabulary(root):
    expected = load(ROOT / 'engines/colmap-runtime.json')['offlineVocabulary']
    dest = contained(root, 'linux/colmap/' + expected['path'])
    dest.parent.mkdir(parents=True, exist_ok=True)
    cache = ROOT / '.cache/native-linux/vocabulary.bin'
    cache.parent.mkdir(parents=True, exist_ok=True)
    if cache.exists():
        verify_vocabulary(cache, expected)
    else:
        temporary = cache.with_suffix('.download')
        subprocess.run(['curl', '--fail', '--location', '--retry', '3',
                        '--connect-timeout', '20', VOCAB_URL, '-o', str(temporary)], check=True)
        verify_vocabulary(temporary, expected)
        temporary.replace(cache)
    shutil.copyfile(cache, dest)


def files(root):
    return sorted(p for p in root.rglob('*') if p.is_file() and p.name != 'NATIVE-BUILD.json')


def record(root, backend):
    build = load(ROOT / 'engines/colmap-build.json')
    brush = load(ROOT / 'engines/manifest.linux.json')['brush']
    metadata = dict(schemaVersion=1, kind='ubuntu22.04-native-source', backend=backend,
                    created=time.time(), architecture=platform.machine(),
                    osRelease=Path('/etc/os-release').read_text(),
                    colmapCommit=build['commit'], brushCommit=brush['commit'],
                    vcpkgCommit=build['vcpkgCommit'], rust='1.95.0',
                    gpuExecutionValidated=False,
                    files={str(p.relative_to(root)): digest(p) for p in files(root)})
    (root / 'NATIVE-BUILD.json').write_text(json.dumps(metadata, indent=2) + '\n')


def verify_inventory(root, metadata):
    if metadata.get('schemaVersion') != 1 or metadata.get('kind') != 'ubuntu22.04-native-source':
        raise ValueError('Not a recorded native build')
    expected = metadata['files']
    actual = {str(p.relative_to(root)) for p in files(root)}
    if actual != set(expected):
        raise ValueError('Native runtime file inventory changed')
    for relative, checksum in expected.items():
        if digest(contained(root, relative)) != checksum:
            raise ValueError(f'Native runtime SHA-256 mismatch: {relative}')


def verify(root):
    metadata = load(root / 'NATIVE-BUILD.json')
    verify_inventory(root, metadata)
    build = load(ROOT / 'engines/colmap-build.json')
    brush = load(ROOT / 'engines/manifest.linux.json')['brush']
    if (metadata['colmapCommit'], metadata['brushCommit'], metadata['vcpkgCommit']) != (build['commit'], brush['commit'], build['vcpkgCommit']):
        raise ValueError('Native build source identity differs from repository locks')
    if metadata['backend'] not in ('cpu', 'cuda') or metadata['architecture'] != 'x86_64':
        raise ValueError('Unsupported native build backend/architecture')
    expected = load(ROOT / 'engines/colmap-runtime.json')['offlineVocabulary']
    verify_vocabulary(root / 'linux/colmap' / expected['path'], expected)
    # ldd checks host ABI/library resolution. The core checks version and required CLI options.
    for binary in [root / 'linux/colmap/bin/colmap', root / 'linux/brush/brush_app']:
        dependencies = subprocess.run(['ldd', str(binary)], capture_output=True, text=True, check=True)
        if 'not found' in dependencies.stdout + dependencies.stderr:
            raise ValueError(f'Unresolved native dependency: {binary}\n{dependencies.stdout}')
    subprocess.run([str(ROOT / 'src-tauri/target/release/splatstudio'),
                    '--engine-dir', str(root), 'health'], check=True)
    print('Verified local build inventory, vocabulary, ABI loading and CLI capabilities. GPU training remains a separate acceptance test.')


def activate(root):
    # Only switch a managed release; never replace or remove a user's directory.
    root = root.resolve(strict=True)
    if root.parent != ROOT / 'engines' or not root.name.startswith('.native-release-'):
        raise ValueError('Activation requires a managed native release directory')
    if not (root / 'NATIVE-BUILD.json').is_file():
        raise ValueError('Missing native build record')
    target = ROOT / 'engines/native-linux'
    if target.exists() and not target.is_symlink():
        raise ValueError(f'Refusing to replace non-symlink directory: {target}')
    temporary = target.with_name(f'.native-link-{os.getpid()}')
    try:
        temporary.symlink_to(root.name, target_is_directory=True)
        temporary.replace(target)
    finally:
        temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['vocabulary', 'record', 'verify', 'activate'])
    parser.add_argument('root', type=Path)
    parser.add_argument('--backend', choices=['cuda', 'cpu'], default='cuda')
    args = parser.parse_args()
    root = args.root.resolve()
    if args.command == 'record':
        record(root, args.backend)
    else:
        globals()[args.command](root)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f'Native runtime error: {error}', file=sys.stderr)
        sys.exit(1)
