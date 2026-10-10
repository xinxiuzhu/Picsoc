#!/usr/bin/env python3
"""Run the real Picsoc service against temporary files, without extra dependencies."""
import argparse
import base64
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request
import zlib


def write_png(path, width=32, height=24, color=(90, 140, 115)):
    def chunk(kind, body):
        return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body))
    rows = b''.join(b'\0' + bytes(color) * width for _ in range(height))
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def write_gif(path):
    # Two one-pixel frames with a loop extension. Preview must retain the original animation.
    header = b'GIF89a' + struct.pack('<HH', 1, 1) + b'\x80\x00\x00\x00\x00\x00\xff\xff\xff'
    loop = b'\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00'
    frame = b'\x21\xf9\x04\x04\x0a\x00\x00\x00\x2c' + struct.pack('<HHHH', 0, 0, 1, 1) + b'\x00\x02\x02'
    path.write_bytes(header + loop + frame + b'\x44\x01\x00' + frame + b'\x4c\x01\x00\x3b')


class SmokeTest(unittest.TestCase):
    binary = None

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='picsoc-api-')
        self.root = Path(self.temp.name)
        self.library = self.root / '中文素材'
        self.library.mkdir()
        (self.library / '子文件夹').mkdir()
        write_png(self.library / '风景.png', 64, 32)
        write_png(self.library / '子文件夹' / '风景.png', 32, 64)
        write_gif(self.library / '动态.gif')
        (self.library / '说明.txt').write_text('Not an image', encoding='utf-8')
        self.outside = self.root / 'outside.png'
        write_png(self.outside)
        try:
            (self.library / 'outside-link.png').symlink_to(self.outside)
        except (OSError, NotImplementedError):
            pass
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            self.port = sock.getsockname()[1]
        self.base = f'http://127.0.0.1:{self.port}'
        self.log = open(self.root / 'service.log', 'w+')
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.process = None
        self.start()

    def start(self, password=None):
        environment = os.environ.copy()
        environment.pop('PICSOC_PASSWORD', None)
        environment.pop('PICSOC_BIND', None)
        environment.pop('PICSOC_DATA_DIR', None)
        self.authorization = None
        if password:
            environment['PICSOC_PASSWORD'] = password
            self.authorization = 'Basic ' + base64.b64encode(f'picsoc:{password}'.encode()).decode()
        self.process = subprocess.Popen([self.binary, '--bind', f'127.0.0.1:{self.port}', '--data-dir', str(self.root / 'data'), '--no-open', '--workers', '1'], stdout=self.log, stderr=subprocess.STDOUT, env=environment)
        for _ in range(150):
            try:
                if self.request('/api/health')['ok']:
                    return
            except (urllib.error.URLError, ConnectionError):
                if self.process.poll() is not None:
                    break
            time.sleep(0.1)
        self.log.flush()
        self.log.seek(0)
        self.fail('Service did not start: ' + self.log.read())

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)

    def tearDown(self):
        self.stop()
        self.log.close()
        self.temp.cleanup()

    def request(self, path, method='GET', body=None, headers=None, raw=False):
        headers = dict(headers or {})
        if self.authorization and 'Authorization' not in headers:
            headers['Authorization'] = self.authorization
        if body is not None:
            body = json.dumps(body, ensure_ascii=False).encode()
            headers['Content-Type'] = 'application/json'
        request = urllib.request.Request(self.base + path, data=body, method=method, headers=headers)
        with self.opener.open(request, timeout=10) as response:
            data = response.read()
            return (response.status, dict(response.headers), data) if raw else json.loads(data)

    def wait_scan(self, library_id, expected):
        for _ in range(200):
            libraries = self.request('/api/libraries')['libraries']
            library = next(item for item in libraries if item['id'] == library_id)
            if library['scan']['state'] == 'error':
                self.fail(library['scan']['error'])
            assets = self.request('/api/assets?limit=100')
            if (library['scan']['state'] == 'idle' and assets['total'] == expected
                    and all(asset['width'] is not None and asset['height'] is not None
                            for asset in assets['assets'])):
                return assets['assets']
            time.sleep(0.1)
        self.fail(f'Scan did not produce {expected} assets')

    def test_real_library_lifecycle(self):
        status, _, html = self.request('/', raw=True)
        self.assertEqual(status, 200)
        self.assertIn(b'<!doctype html', html.lower())
        library = self.request('/api/libraries', 'POST', {'name': '测试素材库', 'path': str(self.library)})
        assets = self.wait_scan(library['id'], 3)
        self.assertEqual(len(assets), 3)
        self.assertEqual({asset['format'] for asset in assets}, {'png', 'gif'})
        self.assertEqual(len({asset['relative_path'] for asset in assets}), 3)
        self.assertTrue(all(asset['name'] != 'outside-link.png' for asset in assets))
        landscape = next(asset for asset in assets if asset['relative_path'] == '风景.png')
        self.assertEqual((landscape['width'], landscape['height']), (64, 32))
        modified = self.request(f"/api/assets/{landscape['id']}", 'PATCH', {'favorite': True, 'tags': ['中文标签', '设计', '中文标签']})
        self.assertTrue(modified['favorite'])
        self.assertEqual(set(modified['tags']), {'中文标签', '设计'})
        tagged = self.request('/api/assets?tag=' + urllib.parse.quote('中文标签'))
        self.assertEqual(tagged['total'], 1)
        searched = self.request('/api/assets?q=' + urllib.parse.quote('风景'))
        self.assertEqual(searched['total'], 2)
        self.assertEqual(self.request('/api/assets?favorite=true')['total'], 1)
        self.assertEqual(self.request('/api/assets?format=gif')['total'], 1)
        tags = self.request('/api/tags')['tags']
        self.assertIn({'name': '中文标签', 'count': 1}, tags)
        original = (self.library / '风景.png').read_bytes()
        status, headers, data = self.request(landscape['original_url'], headers={'Range': 'bytes=0-7'}, raw=True)
        self.assertEqual(status, 206)
        self.assertEqual(data, original[:8])
        self.assertTrue(any(key.lower() == 'content-range' for key in headers))
        status = None
        for _ in range(100):
            try:
                status, headers, thumb = self.request(landscape['thumbnail_url'], raw=True)
                if status == 200:
                    break
            except urllib.error.HTTPError as error:
                if error.code not in (202, 404, 503):
                    raise
            time.sleep(0.1)
        self.assertEqual(status, 200)
        self.assertTrue(thumb.startswith(b'\xff\xd8') or thumb.startswith(b'\x89PNG'))
        gif = next(asset for asset in assets if asset['format'] == 'gif')
        self.assertEqual(self.request(gif['original_url'], raw=True)[2], (self.library / '动态.gif').read_bytes())
        self.request(f"/api/libraries/{library['id']}/scan", 'POST')
        self.wait_scan(library['id'], 3)
        self.assertTrue(self.request(f"/api/assets/{landscape['id']}")['favorite'])
        write_png(self.library / '风景.png', 80, 40)
        self.request(f"/api/libraries/{library['id']}/scan", 'POST')
        self.wait_scan(library['id'], 3)
        changed = self.request(f"/api/assets/{landscape['id']}")
        self.assertEqual((changed['width'], changed['height']), (80, 40))
        self.assertTrue(changed['favorite'])
        self.assertEqual(set(changed['tags']), {'中文标签', '设计'})
        write_png(self.library / '新素材.png', 20, 20)
        self.request(f"/api/libraries/{library['id']}/scan", 'POST')
        self.wait_scan(library['id'], 4)
        self.assertTrue(self.request(f"/api/assets/{landscape['id']}")['favorite'])
        self.stop()
        self.start()
        self.assertEqual(self.request('/api/assets?favorite=true')['total'], 1)
        self.request(f"/api/libraries/{library['id']}", 'DELETE')
        self.assertEqual(self.request('/api/assets')['total'], 0)
        self.assertTrue((self.library / '风景.png').exists())
        self.assertTrue((self.library / '动态.gif').exists())

    @unittest.skipIf(os.name == 'nt', 'Windows symlinks can require elevated privileges')
    def test_indexed_path_cannot_escape_library(self):
        library = self.request('/api/libraries', 'POST', {'name': '路径检查', 'path': str(self.library)})
        assets = self.wait_scan(library['id'], 3)
        asset = next(item for item in assets if item['relative_path'] == '风景.png')
        original = self.library / '风景.png'
        original.unlink()
        original.symlink_to(self.outside)
        with self.assertRaises(urllib.error.HTTPError) as response:
            self.request(asset['original_url'], raw=True)
        self.assertIn(response.exception.code, (400, 403, 404))
        response.exception.close()

    def test_invalid_library(self):
        with self.assertRaises(urllib.error.HTTPError) as response:
            self.request('/api/libraries', 'POST', {'name': '不存在', 'path': str(self.root / 'missing')})
        self.assertIn(response.exception.code, (400, 404, 422))
        self.assertIn('error', json.loads(response.exception.read()))
        response.exception.close()
        with self.assertRaises(urllib.error.HTTPError) as response:
            self.request('/api/assets/999999/original')
        self.assertEqual(response.exception.code, 404)
        response.exception.close()

    def test_existing_folder_navigation(self):
        roots = self.request('/api/directories')
        self.assertTrue(roots['roots'])
        result = self.request('/api/directories?path=' + urllib.parse.quote(str(self.library)))
        # Rust retains Windows' extended-length prefix; compare filesystem identity.
        self.assertTrue(Path(result['path']).samefile(self.library))
        self.assertTrue(Path(result['parent']).samefile(self.root))
        self.assertEqual([item['name'] for item in result['directories']], ['子文件夹'])
        self.assertFalse(result['truncated'])
        with self.assertRaises(urllib.error.HTTPError) as response:
            self.request('/api/directories?path=' + urllib.parse.quote(str(self.outside)))
        self.assertEqual(response.exception.code, 400)
        response.exception.close()

    def test_photos_library_packages_are_skipped_and_exports_remain_importable(self):
        package = self.library / 'Photos Library.PHOTOSLIBRARY'
        originals = package / 'originals'
        originals.mkdir(parents=True)
        write_png(package / 'private.png')
        write_png(originals / 'private-original.png')
        exported = self.root / '照片导出'
        exported.mkdir()
        write_png(exported / '已导出的照片.png')

        picker = self.request('/api/directories?path=' + urllib.parse.quote(str(self.library)))
        self.assertEqual([item['name'] for item in picker['directories']], ['子文件夹'])
        library = self.request('/api/libraries', 'POST', {'name': '普通图片目录', 'path': str(self.library)})
        assets = self.wait_scan(library['id'], 3)
        self.assertEqual({item['relative_path'].replace('\\', '/') for item in assets},
                         {'风景.png', '子文件夹/风景.png', '动态.gif'})
        self.assertEqual(len(self.request('/api/libraries')['libraries']), 1)

        for language, export_hint in [('zh-CN', '导出'), ('en', 'export')]:
            for path in (package, originals):
                for method in ('GET', 'POST'):
                    with self.subTest(language=language, path=path.name, method=method):
                        endpoint = '/api/directories?path=' + urllib.parse.quote(str(path)) if method == 'GET' else '/api/libraries'
                        body = None if method == 'GET' else {'name': '不应添加', 'path': str(path)}
                        with self.assertRaises(urllib.error.HTTPError) as response:
                            self.request(endpoint, method, body, headers={'Accept-Language': language})
                        self.assertEqual(response.exception.code, 400)
                        result = json.loads(response.exception.read())
                        response.exception.close()
                        self.assertEqual(result['code'], 'photos_library_unsupported')
                        self.assertIn(export_hint, result['error'].lower())
                        self.assertEqual([item['id'] for item in self.request('/api/libraries')['libraries']], [library['id']])

        export_library = self.request('/api/libraries', 'POST', {'name': '导出照片', 'path': str(exported)})
        self.wait_scan(export_library['id'], 4)
        exported_assets = self.request('/api/assets?' + urllib.parse.urlencode({'library_id': export_library['id']}))
        self.assertEqual(exported_assets['total'], 1)
        self.assertEqual(exported_assets['assets'][0]['name'], '已导出的照片.png')
        self.assertEqual(self.request(exported_assets['assets'][0]['original_url'], raw=True)[2],
                         (exported / '已导出的照片.png').read_bytes())
        self.assertTrue((package / 'private.png').is_file())
        self.assertTrue((originals / 'private-original.png').is_file())

    def test_batch_and_indexed_subfolders(self):
        nested = self.library / '子文件夹' / '更深目录'
        nested.mkdir()
        write_png(nested / '参考.png')
        originals = {path: path.read_bytes() for path in self.library.rglob('*')
                     if path.is_file() and not path.is_symlink()}
        library = self.request('/api/libraries', 'POST', {'name': '批量整理', 'path': str(self.library)})
        assets = self.wait_scan(library['id'], 4)
        folders_url = f"/api/libraries/{library['id']}/folders"
        root = self.request(folders_url)
        self.assertEqual(root['parent'], '')
        self.assertFalse(root['truncated'])
        self.assertEqual([(item['name'], item['asset_count']) for item in root['folders']], [('子文件夹', 2)])
        child = root['folders'][0]
        self.assertIsNone(child['parent'])
        query = urllib.parse.urlencode({'library_id': library['id'], 'folder': child['path']})
        self.assertEqual(self.request('/api/assets?' + query)['total'], 2)
        children = self.request(folders_url + '?' + urllib.parse.urlencode({'parent': child['path']}))
        self.assertEqual(children['parent'], child['path'])
        self.assertEqual([(item['name'], item['asset_count']) for item in children['folders']], [('更深目录', 1)])
        deep = children['folders'][0]
        self.assertEqual(deep['parent'], child['path'])
        query = urllib.parse.urlencode({'library_id': library['id'], 'folder': deep['path']})
        self.assertEqual(self.request('/api/assets?' + query)['total'], 1)
        self.assertEqual(self.request(folders_url + '?' + urllib.parse.urlencode({'parent': deep['path']}))['folders'], [])

        pngs = [item for item in assets if item['format'] == 'png']
        ids = [item['id'] for item in pngs[:2]]
        result = self.request('/api/assets/batch', 'POST', {
            'ids': ids + ids, 'favorite': True, 'add_tags': [' 批量参考 ', '批量参考', '设计']})
        self.assertEqual(result, {'updated': 2})
        for asset_id in ids:
            updated = self.request(f'/api/assets/{asset_id}')
            self.assertTrue(updated['favorite'])
            self.assertEqual(set(updated['tags']), {'批量参考', '设计'})
        self.assertEqual(self.request('/api/assets?favorite=true')['total'], 2)
        self.request('/api/assets/batch', 'POST', {'ids': ids, 'remove_tags': ['设计', '批量参考'], 'add_tags': ['批量参考']})
        for asset_id in ids:
            self.assertEqual(self.request(f'/api/assets/{asset_id}')['tags'], ['批量参考'])

        gif = next(item for item in assets if item['format'] == 'gif')
        with self.assertRaises(urllib.error.HTTPError) as response:
            self.request('/api/assets/batch', 'POST', {'ids': [gif['id'], 999999], 'favorite': True, 'add_tags': ['不可部分保存']})
        self.assertEqual(response.exception.code, 404)
        response.exception.close()
        unchanged = self.request(f"/api/assets/{gif['id']}")
        self.assertFalse(unchanged['favorite'])
        self.assertEqual(unchanged['tags'], [])
        for path, body in [
            ('/api/assets/batch', {'ids': list(range(1, 502)), 'favorite': True}),
            ('/api/assets/batch', {'ids': ids, 'add_tags': ['长' * 51]}),
            ('/api/assets/batch', {'ids': ids}),
            ('/api/assets?folder=..&library_id=' + str(library['id']), None),
            (folders_url + '?parent=..', None),
        ]:
            with self.assertRaises(urllib.error.HTTPError) as response:
                self.request(path, 'POST' if body is not None else 'GET', body)
            self.assertEqual(response.exception.code, 400)
            response.exception.close()
        self.assertEqual({path: path.read_bytes() for path in originals}, originals)

    def test_password_and_origin_guard(self):
        with self.assertRaises(urllib.error.HTTPError) as response:
            self.request('/api/libraries', 'POST', {'name': '跨站', 'path': str(self.library)},
                         headers={'Origin': 'https://unrelated.example'})
        self.assertEqual(response.exception.code, 403)
        response.exception.close()
        with self.assertRaises(urllib.error.HTTPError) as response:
            self.request('/api/health', headers={'Host': 'unrelated.example'})
        self.assertEqual(response.exception.code, 403)
        response.exception.close()
        self.stop()
        self.start(password='test-only-password')
        for path in ('/', '/api/health', '/api/assets'):
            with self.assertRaises(urllib.error.HTTPError) as response:
                self.request(path, headers={'Authorization': ''}, raw=True)
            self.assertEqual(response.exception.code, 401)
            self.assertIn('Basic', response.exception.headers['WWW-Authenticate'])
            response.exception.close()
        self.assertTrue(self.request('/api/health')['ok'])

    def test_english_api_messages(self):
        roots = self.request('/api/directories', headers={'Accept-Language': 'en-US,en;q=0.9'})
        self.assertTrue(any(item['name'] == 'Home' for item in roots['roots']))
        with self.assertRaises(urllib.error.HTTPError) as response:
            self.request('/api/libraries', 'POST', {'name': 'Missing', 'path': str(self.root / 'missing')},
                         headers={'Accept-Language': 'en'})
        result = json.loads(response.exception.read())
        self.assertIn('code', result)
        self.assertTrue(result['error'].isascii())
        response.exception.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', default='target/debug/picsoc')
    args, test_args = parser.parse_known_args()
    SmokeTest.binary = str(Path(args.binary).resolve())
    unittest.main(argv=['api_smoke.py'] + test_args, verbosity=2)
