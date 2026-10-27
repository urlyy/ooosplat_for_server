#!/usr/bin/env python3
"""Local headless job runner (Python 3.10+, Unix; no third-party packages)."""
import argparse
import fcntl
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent
TERMINAL = {'completed', 'failed', 'cancelled', 'interrupted'}


def write_json(path, value):
    temporary = path.with_suffix('.tmp')
    with temporary.open('w') as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def read_json(path):
    return json.loads(path.read_text())


def job_dir(jobs, job):
    # Never allow a task argument to address a directory outside the job store.
    if str(uuid.UUID(job)) != job:
        raise ValueError('invalid job id')
    path = jobs / job
    if not path.is_dir():
        raise ValueError(f'job not found: {job}')
    return path


def tail(path, limit=128 * 1024):
    try:
        with path.open('rb') as stream:
            stream.seek(0, 2)
            size = stream.tell()
            stream.seek(max(0, size - limit))
            data = stream.read().decode('utf-8', errors='replace')
        return data.splitlines()[1:] if size > limit else data.splitlines()
    except FileNotFoundError:
        return []


def snapshot(path):
    state = read_json(path / 'status.json')
    if state['status'] not in TERMINAL:
        with (path / 'worker.lock').open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                pass
            else:
                state['status'] = 'interrupted'
                state['error'] = '后台进程已退出（可能重启或异常终止）；日志保留，可重新提交。'
    # Ignore incomplete JSON lines while the producer is appending.
    for line in tail(path / 'events.jsonl'):
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get('kind') != 'log':
            state['event'] = event
        if event.get('runtime'):
            state['runtime'] = event['runtime']
        if event.get('acceleration'):
            state['acceleration'] = event['acceleration']
    return state


def submit(args):
    source = Path(args.input).expanduser().resolve(strict=True)
    if not (source.is_file() or source.is_dir()):
        raise ValueError('submit requires a video file or image directory')
    ratio = args.frame_sampling_ratio
    if ratio is not None and (not math.isfinite(ratio) or not .001 <= ratio <= 1):
        raise ValueError('frame sampling ratio must be between 0.001 and 1')
    if ratio is not None and args.no_optimize:
        raise ValueError('--frame-sampling-ratio requires optimization')
    binary = args.binary.expanduser().resolve(strict=True)
    if not os.access(binary, os.X_OK):
        raise ValueError(f'not executable: {binary}')
    engine_dir = args.engine_dir or os.environ.get('OOOSPLAT_ENGINE_DIR')
    if engine_dir is None and (ROOT / 'engines/native-linux').exists():
        engine_dir = ROOT / 'engines/native-linux'
    engine_dir = str(Path(engine_dir).expanduser().resolve(strict=True)) if engine_dir else None
    args.jobs.mkdir(parents=True, exist_ok=True)
    path = args.jobs / str(uuid.uuid4())
    path.mkdir(mode=0o700)
    config = dict(id=path.name, input=str(source), quality=args.quality,
                  optimize=not args.no_optimize, binary=str(binary),
                  engine_dir=engine_dir, video_projection=args.video_projection,
                  no_auto_retry=args.no_auto_retry, frame_sampling_ratio=ratio)
    write_json(path / 'request.json', config)
    write_json(path / 'status.json', dict(config, status='queued', created=time.time()))
    # The lock is acquired before publishing the worker, and inherited through exec.
    with (path / 'worker.lock').open('a') as lock, (path / 'worker.log').open('ab') as log:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                              '--jobs', str(args.jobs), '_worker', path.name, str(lock.fileno())],
                             stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                             start_new_session=True, pass_fds=(lock.fileno(),), cwd=ROOT)
        except Exception as error:
            write_json(path / 'status.json', dict(config, status='failed', error=str(error)))
            raise
    print(path.name)


def worker(args):
    path = job_dir(args.jobs, args.job)
    # Keep inherited ownership for this worker's lifetime, including the queue wait.
    lock = os.fdopen(args.lock_fd, 'a')
    state = read_json(path / 'status.json')
    request = read_json(path / 'request.json')
    cancel = path / 'cancel.request'
    signal.signal(signal.SIGTERM, lambda *_: cancel.touch())
    signal.signal(signal.SIGINT, lambda *_: cancel.touch())
    child = None
    try:
        # Serialize GPU work across submissions in this store. No stale lock on crashes.
        with (args.jobs / 'gpu.lock').open('a') as gpu:
            while True:
                if cancel.exists():
                    state['status'] = 'cancelled'
                    return
                try:
                    fcntl.flock(gpu, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    time.sleep(0.25)
            command = [request['binary']]
            if request['engine_dir']:
                command += ['--engine-dir', request['engine_dir']]
            command += ['generate', request['input'], '--projects-root', str(path / 'projects'),
                        '--quality', request['quality'], '--events', str(path / 'events.jsonl')]
            if request.get('video_projection', 'auto') != 'auto':
                command += ['--video-projection', request['video_projection']]
            if request.get('no_auto_retry'):
                command.append('--no-auto-retry')
            if request.get('frame_sampling_ratio') is not None:
                command += ['--frame-sampling-ratio', str(request['frame_sampling_ratio'])]
            if not request['optimize']:
                command.append('--no-optimize')
            state.update(status='running', started=time.time(), command=command)
            write_json(path / 'status.json', state)
            with (path / 'result.json').open('w') as output, (path / 'pipeline.log').open('w') as log:
                # Inherit gpu.lock: an orphaned core still excludes other jobs.
                child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=output,
                                         stderr=log, pass_fds=(gpu.fileno(),))
                sent_cancel = False
                while child.poll() is None:
                    if cancel.exists() and not sent_cancel:
                        child.terminate()  # Rust cancels and reaps its engine process groups.
                        sent_cancel = True
                        state['status'] = 'cancelling'
                    state['heartbeat'] = time.time()
                    write_json(path / 'status.json', state)
                    time.sleep(0.25)
            state['exit_code'] = child.returncode
            if sent_cancel:
                state['status'] = 'cancelled'
            elif child.returncode != 0:
                state.update(status='failed', error='生成失败；查看 pipeline.log 和项目 logs/。')
            else:
                result = read_json(path / 'result.json')
                if not Path(result['finalPly']).is_file():
                    raise RuntimeError('pipeline returned success without final.ply')
                state.update(status='completed', result=result)
    except Exception as error:
        if child is not None and child.poll() is None:
            child.terminate()
            child.wait()
        state.update(status='failed', error=str(error))
    finally:
        state['finished'] = time.time()
        write_json(path / 'status.json', state)
        lock.close()


def display(path, state, lines):
    print(f"任务: {path.name}  状态: {state['status']}")
    print(f"视频: {state['input']}\n模式: {state['quality']}  自动优化: {state['optimize']}")
    event = state.get('event', {})
    stage_progress = event.get('stageProgress')
    stage_label = f'{stage_progress}%' if stage_progress is not None else '动态估算'
    print(f"阶段: {event.get('stage', '-')}  总进度: {event.get('progress', 0):.2f}%"
          f"  阶段进度: {stage_label}")
    print(f"详情: {event.get('message') or '详见运行详情和最近日志'}")
    if event.get('current') is not None:
        print(f"计数: {event['current']}/{event.get('total')} {event.get('unit', '')}")
    started = state.get('started', state.get('created', time.time()))
    print(f"耗时: {int(state.get('finished', time.time()) - started)} 秒")
    if state.get('runtime'):
        print('运行详情: ' + json.dumps(state['runtime'], ensure_ascii=False))
    if state.get('acceleration'):
        print('加速信息: ' + json.dumps(state['acceleration'], ensure_ascii=False))
    if state.get('result'):
        print('PLY: ' + state['result']['finalPly'])
    if state.get('error'):
        print('错误: ' + state['error'])
    print(f"日志: {path / 'pipeline.log'}")
    if lines:
        print('\n'.join(tail(path / 'pipeline.log')[-lines:]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jobs', type=Path, default=ROOT / 'jobs', help='任务存储目录（同一 GPU 使用同一目录）')
    parser.add_argument('--binary', type=Path, default=ROOT / 'src-tauri/target/release/splatstudio')
    sub = parser.add_subparsers(dest='command', required=True)
    start = sub.add_parser('submit', help='后台提交视频或图片目录，返回任务 ID')
    start.add_argument('input')
    start.add_argument('--quality', choices=['fast', 'balanced', 'high'], default='balanced')
    start.add_argument('--no-optimize', action='store_true')
    start.add_argument('--engine-dir', type=Path)
    start.add_argument('--video-projection', choices=['auto', 'perspective', 'equirectangular'], default='auto')
    start.add_argument('--no-auto-retry', action='store_true')
    start.add_argument('--frame-sampling-ratio', type=float)
    for name in ['status', 'watch']:
        view = sub.add_parser(name)
        view.add_argument('job')
        view.add_argument('--json', action='store_true')
        view.add_argument('--lines', type=int, default=8)
        if name == 'watch':
            view.add_argument('--once', action='store_true')
            view.add_argument('--interval', type=float, default=2)
    stop = sub.add_parser('cancel')
    stop.add_argument('job')
    sub.add_parser('list')
    internal = sub.add_parser('_worker', help='内部后台入口，请使用 submit')
    internal.add_argument('job')
    internal.add_argument('lock_fd', type=int)
    args = parser.parse_args()
    args.jobs = args.jobs.expanduser().resolve()
    if args.command == 'submit':
        submit(args)
    elif args.command == '_worker':
        worker(args)
    elif args.command == 'list':
        for path in sorted(args.jobs.glob('*/status.json')):
            state = snapshot(path.parent)
            print(f"{path.parent.name}  {state['status']:12}  {state['quality']:8}  {state['input']}")
    elif args.command == 'cancel':
        path = job_dir(args.jobs, args.job)
        state = snapshot(path)
        if state['status'] not in TERMINAL:
            (path / 'cancel.request').touch()
            print('取消请求已记录；等待引擎清理退出。')
        else:
            print(state['status'])
    else:
        path = job_dir(args.jobs, args.job)
        if args.lines < 0 or (args.command == 'watch' and args.interval <= 0):
            parser.error('lines must be >= 0 and interval must be > 0')
        while True:
            state = snapshot(path)
            if args.command == 'watch' and not args.once and sys.stdout.isatty():
                print('\033[2J\033[H', end='')
            if args.json:
                print(json.dumps(state, ensure_ascii=False, indent=2))
            else:
                display(path, state, args.lines)
            if args.command == 'status' or args.once or state['status'] in TERMINAL:
                break
            time.sleep(args.interval)


if __name__ == '__main__':
    try:
        main()
    except KeyboardInterrupt:
        pass  # Leaving watch never cancels the background job.
    except (OSError, ValueError, KeyError) as error:
        print(f'error: {error}', file=sys.stderr)
        sys.exit(1)
