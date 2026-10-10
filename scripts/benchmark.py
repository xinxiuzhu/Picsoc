#!/usr/bin/env python3
"""Measure real imports and indexed HTTP queries using disposable PNGs and a native executable."""
import argparse
import json
import math
import os
from pathlib import Path
import platform
import socket
import statistics
import struct
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
import zlib


PAGE_SIZE = 200


def fixture(width, height):
    def chunk(kind, body):
        return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body))
    rows = (b'\0' + bytes((95, 139, 114)) * width) * height
    return (b'\x89PNG\r\n\x1a\n'
            + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def create_fixtures(images, count, width, height, layout):
    """Keep fixture predictions outside the service; never modify its database."""
    landscape = (max(width, height), min(width, height))
    portrait = tuple(reversed(landscape))
    square = (min(width, height), min(width, height))
    variants = (landscape, portrait, square) if layout == 'structured' else ((width, height),)
    blobs = {dimensions: fixture(*dimensions) for dimensions in set(variants)}
    target_ratio = width / height
    combined_orientation = 'landscape' if width != height else 'square'
    expected = {
        'total': count,
        'excluded_map': 0,
        'root_direct': 0,
        'first_collection': 0,
        'first_collection_direct': 0,
        'aspect_ratio': 0,
        'geometry_combined': 0,
    }
    first_collection_children = set()
    root_children = {'empty-folder'}
    original_bytes = 0
    (images / 'empty-folder' / 'empty-child').mkdir(parents=True)
    created_directories = {images}
    for number in range(count):
        dimensions = variants[number % len(variants)]
        is_map = number % 5 == 0
        name = f'{"map" if is_map else "image"}-{number:06d}.png'
        relative_parent = Path('')
        if layout == 'structured' and number % 97:
            collection = f'collection-{number // 500:05d}'
            root_children.add(collection)
            relative_parent = Path(collection)
            if number % 7:
                section = f'section-{number % 4}'
                relative_parent /= section
                if collection == 'collection-00000':
                    first_collection_children.add(section)
            if collection == 'collection-00000':
                expected['first_collection'] += 1
                expected['first_collection_direct'] += int(len(relative_parent.parts) == 1)
        else:
            expected['root_direct'] += 1
        parent = images / relative_parent
        if parent not in created_directories:
            parent.mkdir(parents=True, exist_ok=True)
            created_directories.add(parent)
        blob = blobs[dimensions]
        (parent / name).write_bytes(blob)
        original_bytes += len(blob)
        expected['excluded_map'] += int(not is_map)
        ratio = dimensions[0] / dimensions[1]
        expected['aspect_ratio'] += int(target_ratio * 0.98 <= ratio <= target_ratio * 1.02)
        expected['geometry_combined'] += int(dimensions == landscape and not is_map)
    expected['root_children'] = len(root_children)
    expected['first_collection_children'] = len(first_collection_children)
    expected['combined_orientation'] = combined_orientation
    expected['combined_dimensions'] = list(landscape)
    return expected, original_bytes, sorted(set(variants))


def asset_check(total, offset=0, limit=PAGE_SIZE, exclude_map=False, dimensions=None, parent=None, recursive=True):
    def check(response):
        assert response['total'] == total, (response['total'], total)
        assert response['offset'] == offset, (response['offset'], offset)
        assert response['limit'] == limit, (response['limit'], limit)
        assert len(response['assets']) == min(limit, max(0, total - offset)), (len(response['assets']), total, offset)
        for asset in response['assets']:
            if exclude_map:
                assert 'map' not in asset['name'].lower(), asset['name']
            if dimensions is not None:
                assert [asset['width'], asset['height']] == dimensions, asset
            if parent is not None:
                actual = Path(asset['relative_path']).parent
                # These fixtures run on this host, so both service and predictor use its separator.
                target = Path(parent)
                if recursive:
                    assert target == actual or target in actual.parents, asset['relative_path']
                else:
                    assert actual == target, asset['relative_path']
    return check


def folder_check(direct_count, child_count, empty_name=None):
    def check(response):
        assert response['direct_asset_count'] == direct_count, (response['direct_asset_count'], direct_count)
        assert len(response['folders']) == min(PAGE_SIZE, child_count), (len(response['folders']), child_count)
        assert response['truncated'] == (child_count > PAGE_SIZE), response
        assert bool(response['next_cursor']) == response['truncated'], response
        assert response['separator'] == os.sep, response['separator']
        for folder in response['folders']:
            assert 0 <= folder['direct_asset_count'] <= folder['asset_count'], folder
        if empty_name and child_count <= PAGE_SIZE:
            empty = next(folder for folder in response['folders'] if folder['name'] == empty_name)
            assert empty['asset_count'] == 0 and empty['direct_asset_count'] == 0, empty
            assert empty['has_children'], empty
    return check


def query_plan(library_id, expected, width, height, layout):
    def asset_path(**parameters):
        return '/api/assets?' + urllib.parse.urlencode({'library_id': library_id, 'limit': PAGE_SIZE, **parameters})
    def folder_path(parent=''):
        return f'/api/libraries/{library_id}/folders?' + urllib.parse.urlencode({'parent': parent, 'limit': PAGE_SIZE})
    total = expected['total']
    deep_offset = max(0, total - PAGE_SIZE)
    combined_width, combined_height = expected['combined_dimensions']
    queries = [
        ('assets_first_page', asset_path(sort='modified'), asset_check(total)),
        ('assets_deep_page', asset_path(sort='modified', offset=deep_offset), asset_check(total, deep_offset)),
        ('folders_root', folder_path(), folder_check(expected['root_direct'], expected['root_children'], 'empty-folder')),
        ('assets_root_direct', asset_path(folder='', folder_recursive='false'), asset_check(expected['root_direct'], parent='', recursive=False)),
        ('exclude_map', asset_path(exclude_names='map'), asset_check(expected['excluded_map'], exclude_map=True)),
        ('aspect_ratio', asset_path(aspect_ratio=f'{width}:{height}'), asset_check(expected['aspect_ratio'])),
        ('geometry_and_excluded_name', asset_path(
            orientation=expected['combined_orientation'], aspect_ratio=f'{combined_width}:{combined_height}',
            min_width=combined_width, max_width=combined_width, min_height=combined_height, max_height=combined_height,
            exclude_names='map', sort='pixels'),
         asset_check(expected['geometry_combined'], exclude_map=True, dimensions=expected['combined_dimensions'])),
        ('folders_empty', folder_path('empty-folder'), folder_check(0, 1)),
    ]
    if layout == 'structured' and expected['first_collection']:
        parent = 'collection-00000'
        queries.extend([
            ('folders_collection', folder_path(parent), folder_check(expected['first_collection_direct'], expected['first_collection_children'])),
            ('assets_collection_recursive', asset_path(folder=parent), asset_check(expected['first_collection'], parent=parent)),
            ('assets_collection_direct', asset_path(folder=parent, folder_recursive='false'), asset_check(expected['first_collection_direct'], parent=parent, recursive=False)),
        ])
    return queries


def measure_queries(request, queries, samples):
    timings = {name: [] for name, _, _ in queries}
    # Round-robin sampling avoids giving one endpoint all of the first/cold requests.
    for _ in range(samples):
        for name, path, validate in queries:
            started = time.perf_counter()
            response = request(path)
            elapsed = (time.perf_counter() - started) * 1000
            validate(response)
            timings[name].append(elapsed)
    report = {}
    for name, path, _ in queries:
        values = timings[name]
        ordered = sorted(values)
        report[name] = {
            'samples': len(values),
            'first_ms': round(values[0], 3),
            'p50_ms': round(statistics.median(values), 3),
            'p95_ms': round(ordered[math.ceil(len(ordered) * 0.95) - 1], 3),
            'max_ms': round(max(values), 3),
            'request': path,
        }
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/release/picsoc')
    parser.add_argument('--count', type=int, default=200)
    parser.add_argument('--width', type=int, default=1500)
    parser.add_argument('--height', type=int, default=1000)
    parser.add_argument('--layout', choices=['flat', 'structured'], default='structured')
    parser.add_argument('--query-samples', type=int, default=15)
    parser.add_argument('--timeout', type=int, default=300)
    parser.add_argument('--output', type=Path, help='Also save the complete JSON result to this file.')
    args = parser.parse_args()
    if not (1 <= args.count <= 100000 and 1 <= args.width <= 8192 and 1 <= args.height <= 8192):
        parser.error('Count must be 1–100000 and dimensions 1–8192.')
    if not (1 <= args.query_samples <= 1000 and args.timeout > 0):
        parser.error('Query samples must be 1–1000 and timeout must be positive.')
    binary = str(Path(args.binary).resolve())
    if not Path(binary).is_file():
        parser.error(f'Native executable does not exist: {binary}')
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with tempfile.TemporaryDirectory(prefix='picsoc-benchmark-') as temporary:
        root = Path(temporary)
        images = root / 'images'
        images.mkdir()
        fixture_started = time.monotonic()
        expected, original_bytes, variants = create_fixtures(images, args.count, args.width, args.height, args.layout)
        fixture_seconds = time.monotonic() - fixture_started
        print(f'[benchmark] Created {args.count} real PNG files ({original_bytes} bytes), layout={args.layout}.', file=sys.stderr, flush=True)
        with socket.socket() as socket_handle:
            socket_handle.bind(('127.0.0.1', 0))
            port = socket_handle.getsockname()[1]
        environment = os.environ.copy()
        for variable in ['PICSOC_PASSWORD', 'PICSOC_BIND', 'PICSOC_DATA_DIR']:
            environment.pop(variable, None)
        with (root / 'service.log').open('w+', encoding='utf-8') as log:
            process = subprocess.Popen([binary, '--config', str(root / 'config.toml'), '--bind', f'127.0.0.1:{port}', '--data-dir', str(root / 'data'),
                                        '--workers', '1', '--scan-interval', '0', '--no-open'],
                                       cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT)

            def request(path, body=None):
                message = urllib.request.Request(f'http://127.0.0.1:{port}{path}',
                                                 data=json.dumps(body).encode() if body is not None else None,
                                                 headers={'Content-Type': 'application/json'})
                with opener.open(message, timeout=15) as response:
                    return json.load(response)

            try:
                startup_deadline = time.monotonic() + min(args.timeout, 30)
                while time.monotonic() < startup_deadline:
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
                library = request('/api/libraries', {'name': 'Benchmark', 'path': str(images)})
                library_id = library['id']
                last_progress = started
                indexed_seconds = None
                known = 0
                while time.monotonic() - started < args.timeout:
                    if process.poll() is not None:
                        raise RuntimeError('Service exited during import.')
                    state = request('/api/libraries')['libraries'][0]['scan']
                    if state['state'] == 'error':
                        raise RuntimeError(state['error'])
                    if state['state'] == 'idle':
                        total = request(f'/api/assets?library_id={library_id}&limit=1')['total']
                        if total == args.count:
                            if indexed_seconds is None:
                                indexed_seconds = time.monotonic() - started
                            # A cache file appears before its DB result is committed. Confirm every
                            # dimension is ready via SQL, then walk the cache only once after import.
                            known = request(f'/api/assets?library_id={library_id}&min_width=0&min_height=0&limit=1')['total']
                            if known == args.count:
                                break
                    now = time.monotonic()
                    if now - last_progress >= 5:
                        print(f'[benchmark] scan={state["state"]}, processed={state["processed"]}/{args.count}, dimensions_ready={known}.', file=sys.stderr, flush=True)
                        last_progress = now
                    time.sleep(1)
                else:
                    raise TimeoutError(f'Import/thumbnail generation timed out; dimensions_ready={known}/{args.count}.')
                elapsed = time.monotonic() - started
                cache_count = 0
                cache_bytes = 0
                for path in (root / 'data' / 'thumbnails').rglob('*.png'):
                    cache_count += 1
                    cache_bytes += path.stat().st_size
                assert cache_count == args.count, (cache_count, args.count)
                print(f'[benchmark] Import and thumbnails ready in {elapsed:.3f}s; sampling {args.query_samples} requests per endpoint.', file=sys.stderr, flush=True)
                queries = measure_queries(request, query_plan(library_id, expected, args.width, args.height, args.layout), args.query_samples)
                result = {
                    'platform': platform.platform(), 'assets': args.count,
                    'dimensions': [args.width, args.height], 'dimension_variants': variants,
                    'format': 'solid-color RGB PNG', 'layout': args.layout,
                    'workers': 1, 'fixture_creation_seconds': round(fixture_seconds, 3),
                    'index_ready_seconds': round(indexed_seconds, 3),
                    'import_and_thumbnails_seconds': round(elapsed, 3),
                    'completion_poll_interval_seconds': 1,
                    'original_bytes': original_bytes, 'cache_bytes': cache_bytes,
                    'thumbnail_files': cache_count, 'queries': queries,
                    'validation': 'All endpoint totals, bounded pages, directory counts, name exclusions and combined dimensions matched the generated fixtures.',
                    'scope': 'Disposable solid-color PNGs test asset count and indexed HTTP queries. They do not simulate a 15 GB collection, large-image decoding, GIFs, concurrent clients or production storage.',
                }
            except Exception as error:
                log.flush()
                log.seek(0)
                tail = ''.join(log.readlines()[-12:])
                raise RuntimeError(f'{error}\nLast service log lines:\n{tail}') from error
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
            serialized = json.dumps(result, ensure_ascii=False, indent=2) + '\n'
            if args.output:
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(serialized, encoding='utf-8')
            print(serialized, end='')


if __name__ == '__main__':
    main()
