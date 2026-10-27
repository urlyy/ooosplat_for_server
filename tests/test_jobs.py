import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts/splat.py'
spec = importlib.util.spec_from_file_location('splat', SCRIPT)
splat = importlib.util.module_from_spec(spec)
spec.loader.exec_module(splat)

FAKE = '''#!/usr/bin/env python3
import json, pathlib, signal, sys, time
args = sys.argv
source = pathlib.Path(args[args.index('generate') + 1])
events = pathlib.Path(args[args.index('--events') + 1])
root = pathlib.Path(args[args.index('--projects-root') + 1])
events.write_text(json.dumps(dict(kind='progress', stage='training', progress=42.5, message='training')) + '\\n')
if source.name.startswith('slow'):
    time.sleep(20)
if source.name.startswith('fail'):
    print('synthetic engine failure', file=sys.stderr)
    sys.exit(2)
root.mkdir()
ply = root / 'final.ply'
ply.write_text('test model')
print(json.dumps(dict(finalPly=str(ply))))
'''


class JobsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.jobs = self.root / 'jobs'
        self.binary = self.root / 'fake-core'
        self.binary.write_text(FAKE)
        self.binary.chmod(0o755)
        self.ids = []

    def tearDown(self):
        for job in self.ids:
            self.run_cli('cancel', job)
        for job in self.ids:
            self.wait(job, splat.TERMINAL)
        self.temp.cleanup()

    def run_cli(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), '--jobs', str(self.jobs),
                               '--binary', str(self.binary), *args],
                              text=True, capture_output=True, check=True).stdout.strip()

    def submit(self, name='video.mp4', *extra):
        source = self.root / name
        source.write_bytes(b'fake video')
        job = self.run_cli('submit', str(source), *extra)
        self.ids.append(job)
        return job

    def wait(self, job, states):
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            state = splat.snapshot(self.jobs / job)
            if state['status'] in states:
                return state
            time.sleep(.05)
        self.fail(f'timeout: {state}')

    def test_detached_success_and_progress(self):
        job = self.submit('video with spaces.mp4', '--quality', 'high', '--no-optimize')
        state = self.wait(job, {'completed'})
        self.assertEqual(state['quality'], 'high')
        self.assertFalse(state['optimize'])
        self.assertEqual(state['event']['progress'], 42.5)
        self.assertTrue(Path(state['result']['finalPly']).is_file())
        watched = json.loads(self.run_cli('watch', job, '--once', '--json'))
        self.assertEqual(watched['status'], 'completed')

    def test_projection_and_generation_policy_reach_background_core(self):
        job = self.submit('panorama.mp4', '--video-projection', 'equirectangular',
                          '--no-auto-retry', '--frame-sampling-ratio', '0.2')
        state = self.wait(job, {'completed'})
        command = state['command']
        self.assertEqual(command[command.index('--video-projection') + 1], 'equirectangular')
        self.assertIn('--no-auto-retry', command)
        self.assertEqual(command[command.index('--frame-sampling-ratio') + 1], '0.2')

    def test_image_directory_can_run_as_background_job(self):
        source = self.root / 'images with spaces'
        source.mkdir()
        job = self.run_cli('submit', str(source))
        self.ids.append(job)
        state = self.wait(job, {'completed'})
        self.assertEqual(state['input'], str(source))

    def test_invalid_sampling_rejected_before_job_creation(self):
        source = self.root / 'input.mp4'
        source.write_bytes(b'fake video')
        for flags in [('--frame-sampling-ratio', 'nan'),
                      ('--frame-sampling-ratio', '1.1'),
                      ('--frame-sampling-ratio', '0.2', '--no-optimize')]:
            with self.assertRaises(subprocess.CalledProcessError):
                self.run_cli('submit', str(source), *flags)
        self.assertFalse(self.jobs.exists())

    def test_submitted_job_pins_runtime_symlink(self):
        first = self.root / 'release-first'
        second = self.root / 'release-second'
        first.mkdir()
        second.mkdir()
        active = self.root / 'native-linux'
        active.symlink_to(first, target_is_directory=True)
        with patch.dict(os.environ, {'OOOSPLAT_ENGINE_DIR': str(active)}):
            job = self.submit()
        active.unlink()
        active.symlink_to(second, target_is_directory=True)
        state = self.wait(job, {'completed'})
        self.assertEqual(state['engine_dir'], str(first))

    def test_failure_persisted(self):
        job = self.submit('fail.mp4')
        self.assertEqual(self.wait(job, {'failed'})['exit_code'], 2)
        self.assertIn('synthetic engine failure', (self.jobs / job / 'pipeline.log').read_text())

    def test_running_and_queued_cancellation(self):
        first = self.submit('slow1.mp4')
        self.wait(first, {'running'})
        second = self.submit('slow2.mp4')
        time.sleep(.4)
        self.assertEqual(splat.snapshot(self.jobs / second)['status'], 'queued')
        self.run_cli('cancel', second)
        self.wait(second, {'cancelled'})
        self.run_cli('cancel', first)
        self.wait(first, {'cancelled'})
        third = self.submit()
        self.wait(third, {'completed'})

    def test_orphaned_status_and_partial_event(self):
        job = self.submit()
        self.wait(job, {'completed'})
        path = self.jobs / job
        state = splat.read_json(path / 'status.json')
        state['status'] = 'running'
        splat.write_json(path / 'status.json', state)
        with (path / 'events.jsonl').open('a') as stream:
            stream.write('{"partial":')
        observed = splat.snapshot(path)
        self.assertEqual(observed['status'], 'interrupted')
        self.assertEqual(observed['event']['progress'], 42.5)

    def test_reject_path_traversal(self):
        with self.assertRaises(ValueError):
            splat.job_dir(self.jobs, '../elsewhere')


if __name__ == '__main__':
    unittest.main()
