#!/usr/bin/env python3
"""Measure an actual import using disposable PNG fixtures and a native executable."""
import argparse
import json
import os
from pathlib import Path
import platform
import socket
import struct
import subprocess
import tempfile
import time
import urllib.request
import zlib


def fixture(width, height):
    def chunk(kind, body):
        return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body))
    rows = (b'\0' + bytes((95, 139, 114)) * width) * height
    return (b'\x89PNG\r\n\x1a\n'
            + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/release/picsoc')
    parser.add_argument('--count', type=int, default=200)
    parser.add_argument('--width', type=int, default=1500)
    parser.add_argument('--height', type=int, default=1000)
    parser.add_argument('--timeout', type=int, default=300)
    args = parser.parse_args()
    if not (1 <= args.count <= 10000 and 1 <= args.width <= 8192 and 1 <= args.height <= 8192):
        parser.error('Count must be 1–10000 and dimensions 1–8192.')
    binary = str(Path(args.binary).resolve())
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with tempfile.TemporaryDirectory(prefix='picsoc-benchmark-') as temporary:
        root = Path(temporary)
        images = root / 'images'
        images.mkdir()
        data = fixture(args.width, args.height)
        for number in range(args.count):
            (images / f'image-{number:05d}.png').write_bytes(data)
        with socket.socket() as socket_handle:
            socket_handle.bind(('127.0.0.1', 0))
            port = socket_handle.getsockname()[1]
        environment = os.environ.copy()
        environment.pop('PICSOC_PASSWORD', None)
        environment.pop('PICSOC_BIND', None)
        environment.pop('PICSOC_DATA_DIR', None)
        with (root / 'service.log').open('w+') as log:
            process = subprocess.Popen([binary, '--bind', f'127.0.0.1:{port}', '--data-dir', str(root / 'data'),
                                        '--workers', '1', '--scan-interval', '0', '--no-open'],
                                       env=environment, stdout=log, stderr=subprocess.STDOUT)
            def request(path, body=None):
                message = urllib.request.Request(f'http://127.0.0.1:{port}{path}',
                                                 data=json.dumps(body).encode() if body else None,
                                                 headers={'Content-Type': 'application/json'})
                with opener.open(message, timeout=10) as response:
                    return json.load(response)
            try:
                for _ in range(100):
                    try:
                        request('/api/health')
                        break
                    except OSError:
                        if process.poll() is not None:
                            raise RuntimeError('Service failed to start.')
                        time.sleep(0.1)
                else:
                    raise TimeoutError('Service startup timed out.')
                started = time.monotonic()
                request('/api/libraries', {'name': 'Benchmark', 'path': str(images)})
                while time.monotonic() - started < args.timeout:
                    state = request('/api/libraries')['libraries'][0]['scan']
                    if state['state'] == 'error':
                        raise RuntimeError(state['error'])
                    assets = request('/api/assets?limit=200&sort=modified')
                    # Every fixture must have a cache file; the last API page alone
                    # cannot prove completion because directory traversal order varies.
                    thumbnails = list((root / 'data/thumbnails').rglob('*.png'))
                    if state['state'] == 'idle' and assets['total'] == args.count and len(thumbnails) == args.count and all(
                            item['width'] == args.width and item['height'] == args.height for item in assets['assets']):
                        break
                    time.sleep(0.1)
                else:
                    raise TimeoutError('Import/thumbnail generation timed out.')
                elapsed = time.monotonic() - started
                result = {
                    'platform': platform.platform(), 'assets': args.count,
                    'dimensions': [args.width, args.height], 'format': 'solid-color RGB PNG',
                    'workers': 1, 'import_and_thumbnails_seconds': round(elapsed, 3),
                    'original_bytes': args.count * len(data),
                    'cache_bytes': sum(path.stat().st_size for path in (root / 'data/thumbnails').rglob('*.png')),
                }
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
            try:
                import resource
                peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
                result['peak_child_rss_mib'] = round(peak / (1024 * 1024 if platform.system() == 'Darwin' else 1024), 2)
            except ImportError:
                result['peak_child_rss_mib'] = None
            print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
