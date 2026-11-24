#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Mirror study PDFs to a mounted cloud drive and bring published notes home.

No network credentials, source deletion or cloud deletion. Configuration is
local JSON with source_root, cloud_root and mount_root absolute directory paths.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import tempfile
import time

MAX_FILES = 10000
MAX_FILE_BYTES = 1024 * 1024 * 1024
SKIP_DIRS = {'_PDFLector', 'node_modules', 'target', '__pycache__', 'venv', 'env'}


def stamp(path):
    stat = path.stat()
    return [stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns, stat.st_dev, stat.st_ino]


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        while block := stream.read(65536):
            value.update(block)
    return value.hexdigest()


def safe_parent(path):
    # Check ancestors before mkdir, including an existing destination symlink.
    for item in [path, *path.parents]:
        if item.is_symlink():
            raise ValueError('sync destination crosses a symbolic link')
    path.parent.mkdir(parents=True, exist_ok=True)


_UNSET = object()


def copy_atomic(source, target, expected_target=_UNSET):
    safe_parent(target)
    if expected_target is _UNSET:
        expected_target = stamp(target) if target.exists() else None
    before = stamp(source)
    if before[0] > MAX_FILE_BYTES:
        raise ValueError('sync file exceeds size limit')
    temp_path = None
    try:
        with source.open('rb') as incoming, tempfile.NamedTemporaryFile(
            dir=target.parent, prefix='.pdflector-', suffix='.tmp', delete=False
        ) as outgoing:
            temp_path = Path(outgoing.name)
            count = 0
            while block := incoming.read(65536):
                count += len(block)
                if count > MAX_FILE_BYTES:
                    raise ValueError('sync file exceeds size limit')
                outgoing.write(block)
            outgoing.flush()
            os.fsync(outgoing.fileno())
        if stamp(source) != before:
            raise ValueError('sync source changed while copying; retry next cycle')
        observed_target = stamp(target) if target.exists() else None
        if observed_target != expected_target:
            raise ValueError('sync destination changed while copying; retry next cycle')
        os.replace(temp_path, target)
    finally:
        if temp_path is not None and temp_path.exists():
            temp_path.unlink()


def files(root, outputs=False):
    count = 0
    for current, dirs, names in os.walk(root, followlinks=False):
        dirs[:] = sorted(d for d in dirs if not d.startswith('.')
                         and d not in SKIP_DIRS and not (Path(current)/d).is_symlink()
                         and not (Path(current)/d/'pyvenv.cfg').is_file())
        if len(Path(current).relative_to(root).parts) >= 24:
            dirs[:] = []
        for name in sorted(names):
            path = Path(current)/name
            if name.startswith('.') or path.is_symlink() or not path.is_file():
                continue
            if path.suffix.lower() not in ({'.pdf', '.json'} if outputs else {'.pdf'}):
                continue
            count += 1
            if count > MAX_FILES:
                raise ValueError('sync file count exceeds limit')
            yield path


def transfer(source, target, state, preserve_conflict):
    key = str(target)
    current = stamp(source)
    old = state.get(key, {})
    if target.exists() and old.get('source_stamp') == current and old.get('target_stamp') == stamp(target):
        return False
    source_digest = digest(source)
    target_stamp = stamp(target) if target.exists() else None
    if target.exists():
        target_digest = digest(target)
        if target_digest == source_digest:
            state[key] = {'source_stamp': current, 'target_stamp': stamp(target), 'hash': source_digest}
            return False
        if preserve_conflict and (not old or target_digest != old.get('hash')):
            conflict = target.with_name(f'{target.stem}.desktop-conflict-{target_digest[:16]}{target.suffix}')
            if not conflict.exists():
                copy_atomic(target, conflict)
    copy_atomic(source, target, target_stamp)
    state[key] = {'source_stamp': current, 'target_stamp': stamp(target), 'hash': source_digest}
    return True


def mirror(source, cloud, state):
    source, cloud = Path(source), Path(cloud)
    if not source.is_dir() or not cloud.is_dir():
        raise ValueError('source and cloud directories must be available')
    uploads = downloads = 0
    for path in files(source):
        uploads += transfer(path, cloud/path.relative_to(source), state, True)
    for category in ['Apuntes', 'Respaldo']:
        output_root = cloud/'_PDFLector'/category
        if not output_root.is_dir() or output_root.is_symlink():
            continue
        for path in files(output_root, outputs=True):
            downloads += transfer(path, source/'_PDFLector'/category/path.relative_to(output_root), state, True)
    return uploads, downloads


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--once', action='store_true')
    args = parser.parse_args()
    config = json.loads(args.config.read_text())
    source, cloud, mount = (Path(config[k]) for k in ['source_root', 'cloud_root', 'mount_root'])
    if not all(p.is_absolute() for p in [source, cloud, mount]):
        parser.error('configuration roots must be absolute')
    if cloud == source or cloud.is_relative_to(source) or source.is_relative_to(cloud):
        parser.error('source and cloud trees must be disjoint')
    state_path = args.config.with_name('master-cloud-state.json')
    try:
        state = json.loads(state_path.read_text())
    except FileNotFoundError:
        state = {}
    running = True
    def stop(_signum, _frame):
        nonlocal running
        running = False
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    while running:
        try:
            if not os.path.ismount(mount):
                raise ValueError('cloud drive is not mounted')
            cloud.mkdir(parents=True, exist_ok=True)
            uploads, downloads = mirror(source, cloud, state)
            safe_parent(state_path)
            with tempfile.NamedTemporaryFile(dir=state_path.parent, prefix='.state-', delete=False, mode='w') as stream:
                json.dump(state, stream)
                temporary = stream.name
            os.replace(temporary, state_path)
            if uploads or downloads:
                print(f'Synced {uploads} source PDFs and {downloads} published files', flush=True)
        except (OSError, ValueError) as error:
            # Do not put personal filenames in persistent service logs.
            print(f'Sync unavailable ({type(error).__name__}); retrying', flush=True)
            if args.once:
                raise SystemExit(1) from None
        if args.once:
            return
        for _ in range(30):
            if not running:
                break
            time.sleep(1)


if __name__ == '__main__':
    main()
