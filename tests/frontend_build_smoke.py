#!/usr/bin/env python3
"""Verify Cargo builds and embeds the Web UI from a clean temporary source tree.

Requires the source-build tools (Cargo, Node.js and npm). This is a QA script,
not a Picsoc launcher. Cargo dependencies are reused from --target-dir.
"""
import argparse
import hashlib
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


MARKER = 'picsoc-fresh-cargo-smoke'


def check_build_hook(source, directory, environment):
    """Exercise build errors and prebuilt artifacts without compiling Picsoc."""
    print('build-hook: checking missing npm and prebuilt artifact validation', flush=True)
    executable = directory / ('frontend-build-hook.exe' if os.name == 'nt'
                              else 'frontend-build-hook')
    command = ['rustc', '--edition=2024', '--crate-name', 'picsoc_frontend_build_smoke',
               str(source / 'build.rs'), '-o', str(executable)]
    if os.name == 'nt':
        # Match Picsoc's Windows build without needing runtime DLLs from PATH.
        command += ['-C', 'target-feature=+crt-static']
    compiled = subprocess.run(
        command,
        env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, encoding='utf-8', errors='replace')
    if compiled.returncode != 0:
        raise AssertionError('Could not compile the build hook:\n' + compiled.stdout)

    fixture = directory / 'build-hook-fixture'
    frontend = fixture / 'frontend'
    frontend.mkdir(parents=True)
    output = fixture / 'out'
    output.mkdir()
    isolated = environment.copy()
    isolated.update(PATH='', CARGO_MANIFEST_DIR=str(fixture), OUT_DIR=str(output))
    isolated.pop('PICSOC_FRONTEND_PREBUILT', None)

    def run_hook():
        return subprocess.run([str(executable)], cwd=fixture, env=isolated,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                              text=True, encoding='utf-8', errors='replace')

    result = run_hook()
    if result.returncode == 0 or not all(word in result.stdout.lower()
                                       for word in ('npm', 'node')):
        raise AssertionError('Missing npm did not produce an actionable build failure:\n'
                             + result.stdout)

    dist = frontend / 'dist'
    assets = dist / 'assets'
    assets.mkdir(parents=True)
    (dist / 'index.html').write_text(
        '<!doctype html><html><script type="module" src="/assets/entry.js"></script></html>',
        encoding='utf-8')
    bundle = assets / 'entry.js'
    bundle.write_text('export {};', encoding='utf-8')
    isolated['PICSOC_FRONTEND_PREBUILT'] = '1'
    result = run_hook()
    if result.returncode != 0:
        raise AssertionError('Valid prebuilt frontend required npm:\n' + result.stdout)

    bundle.unlink()
    result = run_hook()
    if result.returncode == 0 or 'frontend/dist' not in result.stdout:
        raise AssertionError('Prebuilt frontend with a missing asset was accepted:\n'
                             + result.stdout)


def copy_sources(source, destination):
    destination.mkdir()
    for name in ('Cargo.toml', 'Cargo.lock', 'build.rs'):
        shutil.copy2(source / name, destination / name)
    shutil.copytree(source / 'src', destination / 'src')
    shutil.copytree(source / 'frontend', destination / 'frontend',
                    ignore=shutil.ignore_patterns('node_modules', 'dist', '*.tsbuildinfo'))
    if (source / '.cargo').is_dir():
        shutil.copytree(source / '.cargo', destination / '.cargo')


def cargo_version(project, environment, label):
    print(f'{label}: cargo run --locked --release -- --version', flush=True)
    result = subprocess.run(
        ['cargo', 'run', '--locked', '--release', '--', '--version'],
        cwd=project, env=environment, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='replace')
    (project.parent / f'{label}.log').write_text(result.stdout, encoding='utf-8')
    if result.returncode != 0:
        raise AssertionError(f'{label} failed:\n{result.stdout[-14000:]}')
    if not any(line.startswith('picsoc ') for line in result.stdout.splitlines()):
        raise AssertionError(f'{label} did not print the executable version:\n{result.stdout[-3000:]}')


def dist_snapshot(dist):
    if not (dist / 'index.html').is_file():
        raise AssertionError('Cargo did not create frontend/dist/index.html')
    files = sorted(path for path in dist.rglob('*') if path.is_file())
    if not any(path.parent.name == 'assets' for path in files):
        raise AssertionError('Cargo did not create frontend assets')
    return {str(path.relative_to(dist)): (path.stat().st_mtime_ns,
                                         hashlib.sha256(path.read_bytes()).hexdigest())
            for path in files}


def assert_marker(dist):
    scripts = list((dist / 'assets').glob('*.js'))
    if not any(MARKER.encode() in path.read_bytes() for path in scripts):
        raise AssertionError('Changed frontend source was not rebuilt into the JavaScript bundle')


class PageAssets(HTMLParser):
    def __init__(self):
        super().__init__()
        self.scripts = []
        self.styles = []

    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        if tag == 'script' and attributes.get('src'):
            self.scripts.append(attributes['src'])
        if tag == 'link' and attributes.get('rel') == 'stylesheet' and attributes.get('href'):
            self.styles.append(attributes['href'])


def check_embedded_http(binary, directory, environment):
    print('http: checking the embedded page and its assets on an isolated port', flush=True)
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    base = f'http://127.0.0.1:{port}'
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def fetch(path):
        address = urllib.parse.urljoin(base + '/', path)
        if urllib.parse.urlsplit(address).netloc != f'127.0.0.1:{port}':
            raise AssertionError('The embedded page referenced an external asset')
        with opener.open(address, timeout=5) as response:
            return response.headers.get_content_type(), response.read()

    with (directory / 'service.log').open('w+', encoding='utf-8') as log:
        process = subprocess.Popen(
            [str(binary), '--bind', f'127.0.0.1:{port}', '--data-dir',
             str(directory / 'data'), '--no-open', '--scan-interval', '0'],
            cwd=directory, env=environment, stdout=log, stderr=subprocess.STDOUT)
        try:
            for _ in range(150):
                try:
                    if json.loads(fetch('/api/health')[1])['ok']:
                        break
                except (urllib.error.URLError, ConnectionError):
                    if process.poll() is not None:
                        break
                    time.sleep(0.1)
            else:
                raise AssertionError('Temporary service did not become ready')
            if process.poll() is not None:
                log.seek(0)
                raise AssertionError('Temporary service exited:\n' + log.read())
            content_type, html = fetch('/')
            if content_type != 'text/html' or b'<script' not in html.lower():
                raise AssertionError('The service did not serve the built frontend page')
            page = PageAssets()
            page.feed(html.decode('utf-8'))
            if not page.scripts:
                raise AssertionError('The page did not reference a JavaScript bundle')
            scripts = [fetch(path) for path in page.scripts]
            if not any(MARKER.encode() in body for _, body in scripts):
                raise AssertionError('The executable embedded an outdated frontend bundle')
            for content_type, body in scripts:
                if content_type not in ('application/javascript', 'text/javascript') or not body:
                    raise AssertionError('A JavaScript asset is missing or returned HTML')
            for path in page.styles:
                content_type, body = fetch(path)
                if content_type != 'text/css' or not body:
                    raise AssertionError('A stylesheet asset is missing or returned HTML')
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)


def main():
    repository = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=repository)
    parser.add_argument('--target-dir', type=Path, default=repository / 'target',
                        help='Reuse Cargo build dependencies from this directory')
    parser.add_argument('--skip-http', action='store_true',
                        help='Only check Cargo builds when localhost sockets are unavailable')
    args = parser.parse_args()
    environment = os.environ.copy()
    for name in ('PICSOC_FRONTEND_PREBUILT', 'CARGO_BUILD_TARGET', 'PICSOC_PASSWORD',
                 'PICSOC_BIND', 'PICSOC_DATA_DIR'):
        environment.pop(name, None)
    target = args.target_dir.resolve()
    environment['CARGO_TARGET_DIR'] = str(target)
    with tempfile.TemporaryDirectory(prefix='picsoc frontend-') as temporary:
        directory = Path(temporary)
        check_build_hook(args.source.resolve(), directory, environment)
        project = directory / '源码'
        copy_sources(args.source.resolve(), project)
        frontend = project / 'frontend'
        dist = frontend / 'dist'
        if dist.exists() or (frontend / 'node_modules').exists():
            raise AssertionError('The temporary source tree was not clean')

        cargo_version(project, environment, 'fresh')
        before = dist_snapshot(dist)
        cargo_version(project, environment, 'unchanged')
        if dist_snapshot(dist) != before:
            raise AssertionError('An unchanged Cargo run rebuilt the frontend')

        source = frontend / 'src' / 'main.tsx'
        with source.open('a', encoding='utf-8') as output:
            output.write(f"\ndocument.documentElement.dataset.picsocBuildSmoke = '{MARKER}';\n")
        cargo_version(project, environment, 'source-changed')
        assert_marker(dist)

        shutil.rmtree(dist)
        cargo_version(project, environment, 'dist-deleted')
        dist_snapshot(dist)
        assert_marker(dist)
        if not args.skip_http:
            binary = target / 'release' / ('picsoc.exe' if os.name == 'nt' else 'picsoc')
            check_embedded_http(binary, directory, environment)
    print('PASS: missing npm, prebuilt validation, fresh Cargo build, unchanged cache, '
          'source changes, deleted dist'
          + (', and embedded HTTP assets' if not args.skip_http else ''), flush=True)


if __name__ == '__main__':
    main()
