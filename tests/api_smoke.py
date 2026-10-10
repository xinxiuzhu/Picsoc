#!/usr/bin/env python3
"""Run the real Picsoc service against temporary files, without extra dependencies."""
import argparse
import base64
from contextlib import closing
import http.cookiejar
import json
import os
from pathlib import Path
import re
import socket
import sqlite3
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
        self.log = open(self.root / 'service.log', 'w+', encoding='utf-8')
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.process = None
        self.start()

    def start(self, password=None, bind=None):
        environment = os.environ.copy()
        environment.pop('PICSOC_PASSWORD', None)
        environment.pop('PICSOC_BIND', None)
        environment.pop('PICSOC_DATA_DIR', None)
        for name in ('PICSOC_MCP_ENABLED', 'PICSOC_PUBLIC_URL', 'PICSOC_MCP_TOKEN', 'PICSOC_MCP_REDIRECT_URIS'):
            environment.pop(name, None)
        self.authorization = None
        if password:
            self.authorization = 'Basic ' + base64.b64encode(f'picsoc:{password}'.encode()).decode()
        data = self.root / 'data'
        data.mkdir(exist_ok=True)
        # JSON string escaping is valid TOML basic-string escaping for these values.
        config = '\n'.join([
            'bind = ' + json.dumps(bind or f'127.0.0.1:{self.port}'),
            'data_dir = ' + json.dumps(str(data), ensure_ascii=False),
            'open_browser = false', 'workers = 1', 'scan_interval = 0',
            'password = ' + json.dumps(password or '', ensure_ascii=False),
            '[mcp]', 'enabled = false', 'public_url = ""', 'token = ""',
            'redirect_uris = []', '',
        ])
        (data / 'config.toml').write_text(config, encoding='utf-8')
        self.process = subprocess.Popen([self.binary, '--bind', bind or f'127.0.0.1:{self.port}', '--data-dir', str(self.root / 'data'), '--no-open', '--workers', '1'], stdout=self.log, stderr=subprocess.STDOUT, env=environment)
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

    def request(self, path, method='GET', body=None, headers=None, raw=False, opener=None, basic=True):
        headers = dict(headers or {})
        if basic and self.authorization and 'Authorization' not in headers:
            headers['Authorization'] = self.authorization
        if body is not None:
            body = json.dumps(body, ensure_ascii=False).encode()
            headers['Content-Type'] = 'application/json'
        request = urllib.request.Request(self.base + path, data=body, method=method, headers=headers)
        with (opener or self.opener).open(request, timeout=10) as response:
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

    def asset_query(self, **parameters):
        parameters = {key: str(value).lower() if isinstance(value, bool) else value
                      for key, value in parameters.items()}
        return self.request('/api/assets?' + urllib.parse.urlencode(parameters))

    def assert_asset_paths(self, expected, **parameters):
        result = self.asset_query(limit=100, **parameters)
        self.assertEqual(result['total'], len(expected), parameters)
        self.assertEqual({item['relative_path'].replace('\\', '/') for item in result['assets']},
                         set(expected), parameters)
        return result

    def assert_bad_query(self, **parameters):
        with self.assertRaises(urllib.error.HTTPError) as response:
            self.asset_query(**parameters)
        self.assertEqual(response.exception.code, 400, parameters)
        response.exception.close()

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

    def test_real_folder_tree_direct_files_empty_folders_and_rescan(self):
        deep = self.library / '子文件夹' / '更深目录' / '末级'
        deep.mkdir(parents=True)
        write_png(deep / '参考.png')
        (self.library / '子文件夹' / '空下层').mkdir()
        sibling = self.library / '子文件夹副本'
        sibling.mkdir()
        write_png(sibling / '旁边.png')
        literal = self.library / '百分%_目录'
        (literal / '中文空目录').mkdir(parents=True)
        write_png(literal / '字面路径.png')
        wildcard = self.library / '百分AB目录'
        wildcard.mkdir()
        write_png(wildcard / '不能混入.png')
        (self.library / '纯空目录' / '空子目录').mkdir(parents=True)
        text_only = self.library / '没有图片'
        text_only.mkdir()
        (text_only / '说明.txt').write_text('Not an image', encoding='utf-8')
        expected_folders = {
            'Cursor A': (0, 0, False), 'Cursor B': (0, 0, False), 'Cursor C': (0, 0, False),
            '子文件夹': (2, 1, True), '子文件夹副本': (1, 1, False),
            '百分%_目录': (1, 1, True), '百分AB目录': (1, 1, False),
            '纯空目录': (0, 0, True), '没有图片': (0, 0, False),
        }
        for name in ('Cursor A', 'Cursor B', 'Cursor C'):
            (self.library / name).mkdir()
        total_assets = 7
        if os.name != 'nt':
            # A backslash is a literal filename character on Unix, not a folder separator.
            backslash_folder = r'反\斜杠目录'
            (self.library / backslash_folder).mkdir()
            write_png(self.library / backslash_folder / '参考.png')
            expected_folders[backslash_folder] = (1, 1, False)
            total_assets += 1

        library = self.request('/api/libraries', 'POST', {'name': '真实目录树', 'path': str(self.library)})
        library_id = library['id']
        self.wait_scan(library_id, total_assets)
        folders_url = f'/api/libraries/{library_id}/folders'

        def folders(parent=''):
            result = self.request(folders_url + '?' + urllib.parse.urlencode({'parent': parent}))
            self.assertEqual(result['parent'].replace('\\', '/'), parent.replace('\\', '/'))
            self.assertFalse(result['truncated'])
            return result, {item['name']: item for item in result['folders']}

        root, children = folders()
        self.assertEqual(root['separator'], os.sep)
        self.assertEqual(root['direct_asset_count'], 2)
        self.assertEqual(set(children), set(expected_folders))
        for name, (total, direct_count, has_children) in expected_folders.items():
            with self.subTest(folder=name):
                child = children[name]
                self.assertIsNone(child['parent'])
                self.assertEqual(child['path'], name)
                self.assertEqual((child['asset_count'], child['direct_asset_count'], child['has_children']),
                                 (total, direct_count, has_children))

        def paginated_folders(parent='', cursor=None, expected_direct_count=2):
            seen = set()
            collected = []
            for _ in range(20):
                parameters = {'parent': parent, 'limit': 2}
                if cursor is not None:
                    parameters['cursor'] = cursor
                page = self.request(folders_url + '?' + urllib.parse.urlencode(parameters))
                self.assertEqual(page['parent'].replace('\\', '/'), parent.replace('\\', '/'))
                self.assertEqual(page['separator'], os.sep)
                self.assertEqual(page['direct_asset_count'], expected_direct_count)
                self.assertLessEqual(len(page['folders']), 2)
                self.assertEqual(page['truncated'], page['next_cursor'] is not None)
                collected.extend(page['folders'])
                next_cursor = page['next_cursor']
                if next_cursor is None:
                    self.assertEqual(len({item['path'] for item in collected}), len(collected))
                    return collected
                self.assertTrue(page['folders'])
                self.assertEqual(next_cursor, page['folders'][-1]['path'])
                self.assertNotIn(next_cursor, seen, 'Folder cursor did not advance')
                seen.add(next_cursor)
                cursor = next_cursor
            self.fail('Folder pagination did not finish')

        pages = paginated_folders()
        self.assertEqual([item['path'] for item in pages], sorted(item['path'] for item in children.values()))
        self.assertEqual({item['name']: item for item in pages}, children)
        first_page = self.request(folders_url + '?' + urllib.parse.urlencode({'limit': 2}))
        deleted_cursor = first_page['next_cursor']
        self.assertEqual(deleted_cursor, 'Cursor B')
        for parameters in [
            {'cursor': '../escape'}, {'cursor': '/absolute'},
            {'cursor': '子文件夹/更深目录'},
            {'parent': '子文件夹', 'cursor': '没有图片'},
            {'parent': '子文件夹', 'cursor': '子文件夹副本/子目录'},
            {'limit': 0}, {'limit': 1001}, {'limit': 'invalid'},
        ]:
            with self.subTest(folder_page_invalid=parameters):
                with self.assertRaises(urllib.error.HTTPError) as response:
                    self.request(folders_url + '?' + urllib.parse.urlencode(parameters))
                self.assertEqual(response.exception.code, 400)
                response.exception.close()

        branch, branch_children = folders('子文件夹')
        self.assertEqual(branch['direct_asset_count'], 1)
        self.assertEqual(set(branch_children), {'更深目录', '空下层'})
        self.assertEqual((branch_children['更深目录']['asset_count'],
                          branch_children['更深目录']['direct_asset_count'],
                          branch_children['更深目录']['has_children']), (1, 0, True))
        self.assertEqual(branch_children['更深目录']['parent'], '子文件夹')
        self.assertEqual({item['name']: item for item in paginated_folders('子文件夹', expected_direct_count=1)},
                         branch_children)
        middle, middle_children = folders('子文件夹/更深目录')
        self.assertEqual(middle['direct_asset_count'], 0)
        self.assertEqual(set(middle_children), {'末级'})
        self.assertEqual((middle_children['末级']['asset_count'],
                          middle_children['末级']['direct_asset_count'],
                          middle_children['末级']['has_children']), (1, 1, False))
        leaf, leaf_children = folders('子文件夹/更深目录/末级')
        self.assertEqual(leaf['direct_asset_count'], 1)
        self.assertFalse(leaf_children)
        empty, empty_children = folders('纯空目录')
        self.assertEqual(empty['direct_asset_count'], 0)
        self.assertEqual(set(empty_children), {'空子目录'})
        self.assertEqual((empty_children['空子目录']['asset_count'],
                          empty_children['空子目录']['direct_asset_count'],
                          empty_children['空子目录']['has_children']), (0, 0, False))

        self.assert_asset_paths({'风景.png', '动态.gif'}, library_id=library_id, folder='', folder_recursive=False)
        self.assertEqual(self.asset_query(library_id=library_id, folder='')['total'], total_assets)
        self.assert_asset_paths({'子文件夹/风景.png', '子文件夹/更深目录/末级/参考.png'},
                                library_id=library_id, folder='子文件夹')
        self.assert_asset_paths({'子文件夹/风景.png'}, library_id=library_id,
                                folder='子文件夹', folder_recursive=False)
        self.assert_asset_paths({'子文件夹/更深目录/末级/参考.png'}, library_id=library_id,
                                folder='子文件夹/更深目录')
        self.assert_asset_paths(set(), library_id=library_id,
                                folder='子文件夹/更深目录', folder_recursive=False)
        self.assert_asset_paths({'百分%_目录/字面路径.png'}, library_id=library_id, folder='百分%_目录')
        self.assert_asset_paths(set(), library_id=library_id, folder='纯空目录')
        if os.name != 'nt':
            literal_result = self.asset_query(library_id=library_id, folder=backslash_folder,
                                              folder_recursive=False)
            self.assertEqual(literal_result['total'], 1)
            self.assertEqual([asset['relative_path'] for asset in literal_result['assets']],
                             [backslash_folder + '/参考.png'])
            backslash_branch, backslash_children = folders(backslash_folder)
            self.assertEqual(backslash_branch['parent'], backslash_folder)
            self.assertEqual(backslash_branch['direct_asset_count'], 1)
            self.assertFalse(backslash_children)

        # Rescanning must update directory metadata as well as the asset list, including deletions.
        (deep / '参考.png').unlink()
        deep.rmdir()
        deep.parent.rmdir()
        (self.library / '纯空目录' / '空子目录').rmdir()
        (self.library / '纯空目录').rmdir()
        (self.library / deleted_cursor).rmdir()
        (self.library / '新空目录' / '尚无图片').mkdir(parents=True)
        write_png(self.library / '子文件夹' / '新增.png')
        self.request(f'/api/libraries/{library_id}/scan', 'POST')
        self.wait_scan(library_id, total_assets)
        _, children = folders()
        self.assertNotIn('纯空目录', children)
        self.assertNotIn(deleted_cursor, children)
        self.assertIn('新空目录', children)
        self.assertEqual((children['新空目录']['asset_count'],
                          children['新空目录']['direct_asset_count'],
                          children['新空目录']['has_children']), (0, 0, True))
        self.assertEqual((children['子文件夹']['asset_count'], children['子文件夹']['direct_asset_count']), (2, 2))
        after_deleted_cursor = paginated_folders(cursor=deleted_cursor)
        expected_after_cursor = {name: child for name, child in children.items() if child['path'] > deleted_cursor}
        self.assertEqual({item['name']: item for item in after_deleted_cursor}, expected_after_cursor)
        self.assertEqual([item['path'] for item in after_deleted_cursor],
                         sorted(item['path'] for item in expected_after_cursor.values()))
        branch, branch_children = folders('子文件夹')
        self.assertEqual(branch['direct_asset_count'], 2)
        self.assertEqual(set(branch_children), {'空下层'})
        self.assert_asset_paths({'子文件夹/风景.png', '子文件夹/新增.png'},
                                library_id=library_id, folder='子文件夹', folder_recursive=False)

        self.stop()
        self.start()
        self.wait_scan(library_id, total_assets)
        _, children = folders()
        self.assertIn('新空目录', children)
        self.assertNotIn('纯空目录', children)
        self.assertEqual(children['新空目录']['asset_count'], 0)

    def test_hidden_directory_scans_and_legacy_tree_pagination(self):
        hidden_images = [self.library / '.git' / 'objects' / 'hidden.png',
                         self.library / '子文件夹' / '.cache' / '深层' / 'hidden.png']
        for path in hidden_images:
            path.parent.mkdir(parents=True)
            write_png(path)
        (self.library / 'only-hidden' / '.git').mkdir(parents=True)
        for name in ('alpha', 'empty'):
            (self.library / name).mkdir()
        # Only directories are excluded; a dot-named image is still an ordinary asset.
        write_png(self.library / '.preview.png')
        library = self.request('/api/libraries', 'POST', {'name': '隐藏目录检查', 'path': str(self.library)})
        library_id = library['id']
        assets = self.wait_scan(library_id, 4)
        self.assertEqual({asset['name'] for asset in assets}, {'风景.png', '动态.gif', '.preview.png'})
        visible_asset = next(asset for asset in assets if asset['relative_path'] == '风景.png')
        self.request(f"/api/assets/{visible_asset['id']}", 'PATCH',
                     {'favorite': True, 'tags': ['保留标签']})

        folders_url = f'/api/libraries/{library_id}/folders'
        expected_names = ['alpha', 'empty', 'only-hidden', '子文件夹']
        tree = self.request(folders_url)
        self.assertEqual([folder['name'] for folder in tree['folders']], expected_names)
        self.assertTrue(all(not folder['has_children'] for folder in tree['folders']))
        self.assertEqual(tree['direct_asset_count'], 3)

        # Simulate an old installation using this test's isolated database. Keeping
        # the service running avoids startup's automatic rescan before the API check.
        rows = []
        legacy_paths = [f'.hidden{index:04}/objects' for index in range(1100)]
        legacy_paths.extend(['.git/objects', '子文件夹/.cache/深层', 'only-hidden/.git'])
        for relative in legacy_paths:
            parts = relative.split('/')
            for depth in range(1, len(parts) + 1):
                native = str(Path(*parts[:depth]))
                parent = str(Path(*parts[:depth - 1])) if depth > 1 else ''
                rows.append((library_id, native, parent, parts[depth - 1], 0))
        # SQLite's transaction context does not close the connection. Close it
        # explicitly so Windows can remove the temporary database in tearDown.
        with closing(sqlite3.connect(self.root / 'data' / 'picsoc.sqlite3', timeout=10)) as connection, connection:
            connection.executemany(
                'INSERT OR IGNORE INTO library_folders(library_id,relative_path,parent,name,seen_generation) '
                'VALUES(?,?,?,?,?)', rows)
            legacy_ids = []
            for path in hidden_images:
                relative = str(path.relative_to(self.library))
                cursor = connection.execute(
                    'INSERT INTO assets(library_id,relative_path,name,format,size,width,height,modified_at,'
                    'mtime_ns,seen_generation,parent_folder) VALUES(?,?,?,?,?,?,?,?,?,?,?)',
                    (library_id, relative, path.name, 'png', path.stat().st_size, 32, 24, 1, 1, 0,
                     str(path.parent.relative_to(self.library))))
                legacy_ids.append(cursor.lastrowid)
        self.assertEqual(self.asset_query(library_id=library_id)['total'], 6)

        collected = []
        cursor = None
        for _ in range(3):
            parameters = {'limit': 2}
            if cursor is not None:
                parameters['cursor'] = cursor
            page = self.request(folders_url + '?' + urllib.parse.urlencode(parameters))
            self.assertEqual(len(page['folders']), 2)
            self.assertTrue(all(not folder['has_children'] for folder in page['folders']))
            self.assertEqual(page['truncated'], page['next_cursor'] is not None)
            collected.extend(folder['name'] for folder in page['folders'])
            cursor = page['next_cursor']
            if cursor is None:
                break
        else:
            self.fail('Visible folder pagination did not finish')
        self.assertEqual(collected, expected_names)
        for parent in ('.git', str(Path('.git') / 'objects'), str(Path('子文件夹') / '.cache')):
            hidden_tree = self.request(folders_url + '?' + urllib.parse.urlencode({'parent': parent}))
            self.assertEqual(hidden_tree['folders'], [])
            self.assertEqual(hidden_tree['direct_asset_count'], 0)
            self.assertFalse(hidden_tree['truncated'])
            self.assertIsNone(hidden_tree['next_cursor'])
        after_hidden_cursor = self.request(folders_url + '?' + urllib.parse.urlencode(
            {'cursor': '.hidden1099', 'limit': 2}))
        self.assertEqual([folder['name'] for folder in after_hidden_cursor['folders']], ['alpha', 'empty'])

        self.request(f'/api/libraries/{library_id}/scan', 'POST')
        self.wait_scan(library_id, 4)
        self.assertEqual(self.asset_query(library_id=library_id)['total'], 4)
        for asset_id in legacy_ids:
            with self.assertRaises(urllib.error.HTTPError) as response:
                self.request(f'/api/assets/{asset_id}')
            self.assertEqual(response.exception.code, 404)
            response.exception.close()
        retained = self.request(f"/api/assets/{visible_asset['id']}")
        self.assertTrue(retained['favorite'])
        self.assertEqual(retained['tags'], ['保留标签'])
        self.assertTrue(all(path.is_file() for path in hidden_images))

        # An explicitly selected hidden root may still be imported by entering its path.
        explicit_root = self.root / '.explicit-library'
        explicit_root.mkdir()
        write_png(explicit_root / '导入.png')
        explicit = self.request('/api/libraries', 'POST',
                                {'name': '显式根目录', 'path': str(explicit_root)})
        self.wait_scan(explicit['id'], 5)
        self.assertEqual(self.asset_query(library_id=explicit['id'])['total'], 1)

    def test_advanced_image_filters_combination_pagination_and_validation(self):
        directory = self.library / '筛选'
        (directory / '深层').mkdir(parents=True)
        dimensions = {
            '筛选/横图16比9.png': (160, 90), '筛选/接近16比9.png': (158, 90),
            '筛选/超出下限.png': (156, 90), '筛选/超出上限.png': (164, 90),
            '筛选/竖图9比16.png': (90, 160), '筛选/正方形.png': (100, 100),
            '筛选/大图16比9.png': (640, 360), '筛选/深层/另一个16比9.png': (160, 90),
            '筛选/比例下边界.png': (196, 100), '筛选/比例上边界.png': (204, 100),
            '风景.png': (64, 32), '子文件夹/风景.png': (32, 64), '动态.gif': (1, 1),
        }
        for relative, (width, height) in dimensions.items():
            if relative.startswith('筛选/'):
                write_png(self.library / relative, width, height)
        library = self.request('/api/libraries', 'POST', {'name': '筛选测试', 'path': str(self.library)})
        library_id = library['id']
        assets = self.wait_scan(library_id, len(dimensions))
        by_path = {asset['relative_path'].replace('\\', '/'): asset for asset in assets}
        for relative, (width, height) in dimensions.items():
            self.assertEqual((by_path[relative]['width'], by_path[relative]['height']), (width, height))

        for orientation, predicate in [('landscape', lambda w, h: w > h),
                                        ('portrait', lambda w, h: w < h),
                                        ('square', lambda w, h: w == h)]:
            with self.subTest(orientation=orientation):
                self.assert_asset_paths({name for name, (w, h) in dimensions.items() if predicate(w, h)},
                                        library_id=library_id, orientation=orientation)
        ratio_paths = {'筛选/横图16比9.png', '筛选/接近16比9.png',
                       '筛选/大图16比9.png', '筛选/深层/另一个16比9.png'}
        self.assert_asset_paths(ratio_paths, library_id=library_id, aspect_ratio='16:9')
        self.assert_asset_paths({'筛选/竖图9比16.png'}, library_id=library_id, aspect_ratio='9:16')
        self.assert_asset_paths({'筛选/正方形.png', '动态.gif'}, library_id=library_id, aspect_ratio='1:1')
        self.assert_asset_paths({'风景.png', '筛选/比例下边界.png', '筛选/比例上边界.png'},
                                library_id=library_id, aspect_ratio='2:1')
        self.assert_asset_paths(ratio_paths, library_id=library_id, aspect_ratio='1.6:0.9')

        for key, predicate in [
            ('min_width', lambda w, h: w >= 160), ('max_width', lambda w, h: w <= 160),
            ('min_height', lambda w, h: h >= 100), ('max_height', lambda w, h: h <= 100),
        ]:
            threshold = 160 if 'width' in key else 100
            with self.subTest(dimension=key):
                self.assert_asset_paths({name for name, (w, h) in dimensions.items() if predicate(w, h)},
                                        library_id=library_id, **{key: threshold})
        self.assert_asset_paths({'筛选/横图16比9.png', '筛选/深层/另一个16比9.png'},
                                library_id=library_id, min_width=160, max_width=160,
                                min_height=90, max_height=90)
        self.assert_asset_paths(set(), library_id=library_id, min_width=10_000)

        sizes = {name: (self.library / name).stat().st_size for name in dimensions}
        threshold = sorted(set(sizes.values()))[len(set(sizes.values())) // 2]
        self.assertTrue(any(size < threshold for size in sizes.values()))
        self.assertTrue(any(size > threshold for size in sizes.values()))
        for key, predicate in [('min_size', lambda size: size >= threshold),
                               ('max_size', lambda size: size <= threshold)]:
            with self.subTest(size=key):
                self.assert_asset_paths({name for name, size in sizes.items() if predicate(size)},
                                        library_id=library_id, **{key: threshold})
        self.assert_asset_paths({name for name, size in sizes.items() if size == threshold},
                                library_id=library_id, min_size=threshold, max_size=threshold)
        self.assert_asset_paths(set(), library_id=library_id, max_size=0)
        for sort, field, descending in [('size', 'size', True), ('size_asc', 'size', False),
                                         ('width', 'width', True), ('height', 'height', True),
                                         ('pixels', None, True)]:
            with self.subTest(sort=sort):
                result = self.asset_query(library_id=library_id, sort=sort, limit=100)
                self.assertEqual(result['total'], len(dimensions))
                values = [asset[field] if field else asset['width'] * asset['height']
                          for asset in result['assets']]
                self.assertEqual(values, sorted(values, reverse=descending))

        selected = {'筛选/横图16比9.png', '筛选/深层/另一个16比9.png'}
        for name in selected:
            self.request(f"/api/assets/{by_path[name]['id']}", 'PATCH', {'favorite': True, 'tags': ['中文筛选']})
        combined = dict(library_id=library_id, folder='筛选', folder_recursive=True,
                        orientation='landscape', aspect_ratio='16:9', min_width=160, max_width=200,
                        min_height=80, max_height=100, min_size=0, max_size=max(sizes.values()),
                        favorite=True, tag='中文筛选', format='png', q='16比9', sort='name')
        self.assert_asset_paths(selected, **combined)
        first = self.asset_query(**combined, limit=1, offset=0)
        second = self.asset_query(**combined, limit=1, offset=1)
        end = self.asset_query(**combined, limit=1, offset=2)
        for result, offset in [(first, 0), (second, 1), (end, 2)]:
            self.assertEqual((result['total'], result['limit'], result['offset']), (2, 1, offset))
        self.assertEqual(len(first['assets']), 1)
        self.assertEqual(len(second['assets']), 1)
        self.assertEqual({item['relative_path'].replace('\\', '/')
                          for item in first['assets'] + second['assets']}, selected)
        self.assertEqual(end['assets'], [])
        combined['folder_recursive'] = False
        self.assert_asset_paths({'筛选/横图16比9.png'}, **combined)
        combined['orientation'] = 'portrait'
        self.assert_asset_paths(set(), **combined)

        for parameters in [
            {'orientation': 'diagonal'}, {'sort': 'unsupported'}, {'format': 'unsupported'},
            {'aspect_ratio': 'wide'}, {'aspect_ratio': '0:9'},
            {'aspect_ratio': '16:0'}, {'aspect_ratio': '-1:1'}, {'aspect_ratio': 'nan:1'},
            {'aspect_ratio': 'inf:1'}, {'aspect_ratio': '16:9:1'},
            {'min_width': -1}, {'max_height': -1}, {'min_height': '1.5'},
            {'max_width': 4_294_967_296}, {'min_width': 200, 'max_width': 100},
            {'min_height': 200, 'max_height': 100}, {'min_size': -1}, {'max_size': -1},
            {'min_size': 200, 'max_size': 100}, {'min_size': 9_223_372_036_854_775_808},
            {'folder_recursive': 'not-a-bool'}, {'folder': '../escape'},
            {'folder': '/absolute'}, {'folder': '筛选/../子文件夹'},
        ]:
            with self.subTest(invalid=parameters):
                self.assert_bad_query(library_id=library_id, **parameters)
        self.assert_bad_query(folder='筛选')
        self.assert_bad_query(folder_recursive=False)

    def test_startup_log_reports_actual_listener(self):
        self.stop()
        self.log.seek(0, os.SEEK_END)
        offset = self.log.tell()
        self.start(password='listener-test-password', bind=f'0.0.0.0:{self.port}')
        self.log.flush()
        self.log.seek(offset)
        output = self.log.read()
        self.assertIn(f'0.0.0.0:{self.port}', output)
        self.assertIn(f'127.0.0.1:{self.port}', output)

    def test_filename_exclusions_are_literal_and_compose_with_other_filters(self):
        directory = self.root / '噪音过滤素材'
        names = {
            'normal.png', 'map.png', 'MAP-preview.png', 'roadmap.png', 'bumP.png',
            'roughness.png', '贴图_法线.png', '百分%素材.png', '百分XX素材.png',
            'under_score.png', 'underscore.png', '含map的目录/normal.png', '竖图.png',
        }
        if os.name != 'nt':
            names.add(r'反\斜杠.png')
        for relative in names:
            path = directory / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            write_png(path, 90 if relative == '竖图.png' else 160,
                      160 if relative == '竖图.png' else 90)
        library = self.request('/api/libraries', 'POST', {'name': '文件名排除测试', 'path': str(directory)})
        library_id = library['id']
        assets = self.wait_scan(library_id, len(names))
        by_path = {asset['relative_path'].replace(os.sep, '/'): asset for asset in assets}

        def expected_without(*words):
            # Test names use only ASCII and Chinese; ASCII folding reproduces the documented matching.
            return {name for name in names if not any(
                word.lower() in Path(name).name.lower() for word in words)}

        def check_exclusions(value, expected):
            result = self.asset_query(library_id=library_id, exclude_names=value, limit=100)
            self.assertEqual(result['total'], len(expected), value)
            # Preserve literal backslashes on Unix while normalizing Windows directory separators.
            self.assertEqual({asset['relative_path'].replace(os.sep, '/') for asset in result['assets']},
                             expected, value)

        check_exclusions('', names)
        check_exclusions('\n \n\t', names)
        for value, words in [
            (' map ', ['map']), ('MAP', ['map']), ('法线', ['法线']),
            (' map \n\nbump\n法线\nmap ', ['map', 'bump', '法线']),
            (' % \n', ['%']), ('_', ['_']), ('%_', ['%_']), ('\\', ['\\']),
        ]:
            with self.subTest(exclusions=value):
                check_exclusions(value, expected_without(*words))
        check_exclusions(' map \n' * 51, expected_without('map'))
        check_exclusions('\n'.join(f'missing-{number}' for number in range(50)), names)
        check_exclusions('图' * 100, names)
        boundary_words = '\n'.join(f'{number:02d}' + '图' * 98 for number in range(13))
        boundary_value = boundary_words + ' ' * (4096 - len(boundary_words.encode('utf-8')))
        self.assertEqual(len(boundary_value.encode('utf-8')), 4096)
        check_exclusions(boundary_value, names)

        # The normal filenames remain visible even though their directory or tags contain "map".
        combined_names = {'normal.png', '含map的目录/normal.png', 'map.png', '竖图.png'}
        for relative in combined_names:
            self.request(f"/api/assets/{by_path[relative]['id']}", 'PATCH',
                         {'favorite': True, 'tags': ['map', '中文标签']})
        combined = dict(library_id=library_id, folder='', folder_recursive=True,
                        exclude_names='map', favorite=True, tag='map', q='map', format='png',
                        orientation='landscape', aspect_ratio='16:9', min_width=160,
                        max_width=160, min_height=90, max_height=90, sort='name')
        expected_combined = {'normal.png', '含map的目录/normal.png'}
        self.assert_asset_paths(expected_combined, **combined)
        pages = [self.asset_query(**combined, limit=1, offset=offset) for offset in range(3)]
        for offset, result in enumerate(pages):
            self.assertEqual((result['total'], result['offset'], result['limit']), (2, offset, 1))
        self.assertEqual([len(page['assets']) for page in pages], [1, 1, 0])
        self.assertEqual({asset['relative_path'].replace(os.sep, '/')
                          for page in pages for asset in page['assets']}, expected_combined)

        for value in ['图' * 101, '\n'.join(f'unique-{number}' for number in range(51)),
                      boundary_value + ' ', 'map\0normal']:
            with self.subTest(invalid_exclusions=value[:30]):
                with self.assertRaises(urllib.error.HTTPError) as response:
                    self.asset_query(library_id=library_id, exclude_names=value)
                self.assertEqual(response.exception.code, 400)
                self.assertEqual(json.loads(response.exception.read())['code'], 'invalid_excluded_names')
                response.exception.close()

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
        status, _, html = self.request('/', basic=False, raw=True)
        self.assertEqual(status, 200)
        self.assertIn(b'<!doctype html', html.lower())
        for path in ('/api/health', '/api/assets', '/api/stats', '/api/libraries', '/api/tags'):
            with self.assertRaises(urllib.error.HTTPError) as response:
                self.request(path, headers={'Authorization': ''}, raw=True)
            self.assertEqual(response.exception.code, 401)
            self.assertNotIn('WWW-Authenticate', response.exception.headers)
            self.assertIn('error', json.loads(response.exception.read()))
            response.exception.close()
        self.assertTrue(self.request('/api/health')['ok'])

    def test_cookie_login_logout_restart_and_basic_compatibility(self):
        public_status = {'password_required': False, 'authenticated': True}
        self.assertEqual(self.request('/api/auth/status', basic=False), public_status)
        self.assertEqual(self.request('/api/auth/login', 'POST', {'password': ''}, basic=False), public_status)
        self.assertEqual(self.request('/api/auth/logout', 'POST', basic=False), public_status)
        library = self.request('/api/libraries', 'POST', {'name': '登录后访问素材', 'path': str(self.library)})
        assets = self.wait_scan(library['id'], 3)
        image = next(item for item in assets if item['relative_path'] == '风景.png')
        original = (self.library / '风景.png').read_bytes()
        self.stop()
        password = 'browser-cookie-test-password'
        self.start(password=password)

        jar = http.cookiejar.CookieJar()
        browser = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                               urllib.request.HTTPCookieProcessor(jar))

        def browser_request(path, method='GET', body=None, headers=None, raw=False):
            return self.request(path, method, body, headers, raw, opener=browser, basic=False)

        def denied(path, method='GET', body=None, headers=None, expected=401, opener=browser):
            with self.assertRaises(urllib.error.HTTPError) as response:
                self.request(path, method, body, headers, opener=opener, basic=False)
            self.assertEqual(response.exception.code, expected)
            self.assertNotIn('WWW-Authenticate', response.exception.headers)
            self.assertTrue(response.exception.headers['Content-Type'].startswith('application/json'))
            result = json.loads(response.exception.read())
            self.assertIn('error', result)
            response.exception.close()
            return result

        locked_status = {'password_required': True, 'authenticated': False}
        unlocked_status = {'password_required': True, 'authenticated': True}
        self.assertEqual(browser_request('/api/auth/status'), locked_status)
        status, _, html = browser_request('/', raw=True)
        self.assertEqual(status, 200)
        self.assertIn(b'<!doctype html', html.lower())
        status, _, head_body = browser_request('/', method='HEAD', raw=True)
        self.assertEqual(status, 200)
        self.assertEqual(head_body, b'')
        web_assets = re.findall(r'(?:src|href)="(/assets/[^\"]+)"', html.decode('utf-8'))
        self.assertTrue(web_assets, 'Anonymous login page must load its JavaScript and styles')
        for path in web_assets:
            with self.subTest(anonymous_web_asset=path):
                status, _, content = browser_request(path, raw=True)
                self.assertEqual(status, 200)
                self.assertTrue(content)
        for path in ('/api/assets', '/api/stats', '/api/libraries', '/api/tags',
                     image['original_url'], image['thumbnail_url']):
            with self.subTest(anonymous=path):
                denied(path)
        error = denied('/api/auth/login', 'POST', {'password': 'wrong-password'})
        self.assertEqual(error['code'], 'invalid_password')
        self.assertFalse(list(jar))
        for body in ({}, {'password': 123}):
            denied('/api/auth/login', 'POST', body, expected=400)
        denied('/api/auth/login', 'POST', {'password': password},
               headers={'Origin': 'https://unrelated.example'}, expected=403)
        self.assertEqual(browser_request('/api/auth/status'), locked_status)
        self.assertFalse(list(jar))

        status, headers, data = browser_request('/api/auth/login', 'POST', {'password': password},
                                                headers={'Origin': self.base}, raw=True)
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(data), unlocked_status)
        cookie_header = next(value for key, value in headers.items() if key.lower() == 'set-cookie')
        attributes = cookie_header.lower()
        for attribute in ('picsoc_session=', 'httponly', 'samesite=strict', 'path=/', 'max-age=86400'):
            self.assertIn(attribute, attributes)
        self.assertEqual([cookie.name for cookie in jar], ['picsoc_session'])
        old_session = '; '.join(f'{cookie.name}={cookie.value}' for cookie in jar)
        self.assertEqual(browser_request('/api/auth/status'), unlocked_status)
        self.assertEqual(browser_request('/api/assets')['total'], 3)
        self.assertEqual(browser_request('/api/stats')['total_assets'], 3)
        self.assertEqual([item['id'] for item in browser_request('/api/libraries')['libraries']], [library['id']])
        self.assertEqual(browser_request(image['original_url'], raw=True)[2], original)
        status, _, thumbnail = browser_request(image['thumbnail_url'], raw=True)
        self.assertEqual(status, 200)
        self.assertTrue(thumbnail.startswith(b'\xff\xd8') or thumbnail.startswith(b'\x89PNG'))
        denied('/api/auth/logout', 'POST', headers={'Origin': 'https://unrelated.example'}, expected=403)
        self.assertEqual(browser_request('/api/auth/status'), unlocked_status)

        status, headers, data = browser_request('/api/auth/logout', 'POST',
                                                headers={'Origin': self.base}, raw=True)
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(data), locked_status)
        cookie_header = next(value for key, value in headers.items() if key.lower() == 'set-cookie')
        self.assertIn('max-age=0', cookie_header.lower())
        self.assertFalse(list(jar))
        self.assertEqual(browser_request('/api/auth/status'), locked_status)
        denied('/api/assets')
        # Replaying the former cookie must fail even if the client ignores the deletion header.
        denied('/api/assets', headers={'Cookie': old_session}, opener=self.opener)

        self.assertEqual(browser_request('/api/auth/login', 'POST', {'password': password}), unlocked_status)
        self.assertTrue(list(jar))
        restart_session = '; '.join(f'{cookie.name}={cookie.value}' for cookie in jar)
        self.stop()
        self.start(password=password)
        self.assertEqual(browser_request('/api/auth/status'), locked_status)
        denied('/api/assets')
        denied('/api/assets', headers={'Cookie': restart_session}, opener=self.opener)
        # The existing API authentication mechanism remains available to scripts after restart.
        self.assertEqual(self.request('/api/assets')['total'], 3)
        self.assertEqual(self.request(image['original_url'], raw=True)[2], original)
        self.assertEqual(browser_request('/api/auth/login', 'POST', {'password': password}), unlocked_status)
        self.assertEqual(browser_request('/api/assets')['total'], 3)

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
