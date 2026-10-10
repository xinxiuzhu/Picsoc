// Exercise the real Rust service over HTTP. No Node dependencies are required.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, writeFile, readFile, rm, stat } from 'node:fs/promises';
import net from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { deflateSync } from 'node:zlib';
import { pathToFileURL } from 'node:url';

const binary = path.resolve(process.argv[2] || 'target/debug/picsoc');
const root = await mkdtemp(path.join(tmpdir(), 'picsoc-mcp-'));
const data = path.join(root, 'data');
const library = path.join(root, '中文素材');
await mkdir(library); await mkdir(data);
const socket = net.createServer(); socket.listen(0, '127.0.0.1'); await once(socket, 'listening');
const port = socket.address().port; await new Promise(resolve => socket.close(resolve));
const base = `http://127.0.0.1:${port}`;
const password = 'picsoc-smoke-password';
const token = 'picsoc-smoke-independent-token-0123456789';
const basic = `Basic ${Buffer.from(`picsoc:${password}`).toString('base64')}`;
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
let processHandle, logs = '', rpcId = 0;

function crc32(bytes) { let crc = 0xffffffff; for (const byte of bytes) { crc ^= byte; for (let i = 0; i < 8; i++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0); } return (crc ^ 0xffffffff) >>> 0; }
function png(width, height, rgba) {
  const chunk = (name, bytes) => { const kind = Buffer.from(name), length = Buffer.alloc(4), crc = Buffer.alloc(4); length.writeUInt32BE(bytes.length); crc.writeUInt32BE(crc32(Buffer.concat([kind, bytes]))); return Buffer.concat([length, kind, bytes, crc]); };
  const header = Buffer.alloc(13); header.writeUInt32BE(width); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 6;
  const rows = Buffer.alloc((width * 4 + 1) * height); for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) Buffer.from(rgba).copy(rows, y * (width * 4 + 1) + x * 4 + 1);
  return Buffer.concat([Buffer.from('89504e470d0a1a0a', 'hex'), chunk('IHDR', header), chunk('IDAT', deflateSync(rows)), chunk('IEND', Buffer.alloc(0))]);
}
function assertPng(bytes, width, height) { assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a'); if (width) assert.equal(bytes.readUInt32BE(16), width); if (height) assert.equal(bytes.readUInt32BE(20), height); }
function assertSameScene(actual, expected) {
  // Rendering uses f32 for these fields; direct JSON and MCP Value serialization may spell them differently.
  const normalize = (value, key = '') => {
    if (typeof value === 'number' && ['opacity', 'font_size', 'line_height'].includes(key)) return Math.fround(value);
    if (Array.isArray(value)) return value.map(item => normalize(item));
    if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([name, item]) => [name, normalize(item, name)]));
    return value;
  };
  assert.deepEqual(normalize(actual), normalize(expected));
}
async function start(enabled = true) {
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('PICSOC_')));
  Object.assign(env, { PICSOC_PASSWORD: password, PICSOC_MCP_ENABLED: String(enabled), PICSOC_PUBLIC_URL: enabled ? base : '', PICSOC_MCP_TOKEN: enabled ? token : '', PICSOC_MCP_REDIRECT_URIS: '' });
  processHandle = spawn(binary, ['--bind', `127.0.0.1:${port}`, '--data-dir', data, '--no-open', '--scan-interval', '0'], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  processHandle.stdout.on('data', bytes => { logs += bytes; }); processHandle.stderr.on('data', bytes => { logs += bytes; });
  for (let i = 0; i < 150; i++) { if (processHandle.exitCode !== null) throw new Error(`Service stopped: ${logs}`); try { const response = await fetch(`${base}/api/auth/status`); if (response.ok) return; } catch {} await wait(100); }
  throw new Error(`Service did not start: ${logs}`);
}
async function stop() {
  if (!processHandle || processHandle.exitCode !== null) return;
  const handle = processHandle, exited = once(handle, 'exit');
  const timer = setTimeout(() => { if (handle.exitCode === null) handle.kill('SIGKILL'); }, 10000);
  try { handle.kill('SIGTERM'); await exited; }
  finally { clearTimeout(timer); processHandle = undefined; }
}
async function api(url, body) { const response = await fetch(`${base}${url}`, { headers: { Authorization: basic, 'Content-Type': 'application/json' }, ...(body ? { method: 'POST', body: JSON.stringify(body) } : {}) }); assert.ok(response.ok, `${url}: ${response.status} ${await response.clone().text()}`); return response.json(); }
async function rpc(method, params, credential = token) { return fetch(`${base}/mcp`, { method: 'POST', headers: { Authorization: `Bearer ${credential}`, Accept: 'application/json, text/event-stream', 'Content-Type': 'application/json', 'MCP-Protocol-Version': '2025-11-25' }, body: JSON.stringify({ jsonrpc: '2.0', id: ++rpcId, method, params }) }); }
async function call(name, args = {}) { const response = await rpc('tools/call', { name, arguments: args }); assert.equal(response.status, 200); const value = await response.json(); assert.equal(value.error, undefined, JSON.stringify(value)); return value.result; }
async function finished(jobId) { for (let i = 0; i < 150; i++) { const result = await call('get_render', { job_id: jobId }); assert.equal(result.isError, false, JSON.stringify(result)); const job = result.structuredContent.job; if (job.status === 'succeeded') return result; if (job.status === 'failed') throw new Error(job.error); await wait(100); } throw new Error('Rendering timed out'); }

try {
  await writeFile(path.join(library, '蓝色边框.png'), png(48, 24, [40, 120, 240, 128]));
  await writeFile(path.join(library, 'map噪音.png'), png(24, 48, [120, 200, 60, 255]));
  await start(false); assert.equal((await rpc('ping', {})).status, 404); await stop();
  await start();
  const init = await (await rpc('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'Picsoc smoke', version: '1' } })).json(); assert.equal(init.result.protocolVersion, '2025-11-25');
  const login = await fetch(`${base}/api/auth/login`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ password }) }); const cookie = login.headers.get('set-cookie').split(';')[0];
  assert.equal((await fetch(`${base}/mcp`, { method: 'POST', headers: { Cookie: cookie, Accept: 'application/json, text/event-stream', 'Content-Type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'ping' }) })).status, 401);
  const imported = await api('/api/libraries', { name: '中文素材', path: library });
  let assets;
  for (let i = 0; i < 100; i++) { assets = await call('search_assets', { library_id: imported.id, exclude_names: 'map', orientation: 'landscape' }); if (assets.structuredContent?.assets.length === 1) break; await wait(100); }
  assert.equal(assets.structuredContent.assets.length, 1); const assetId = assets.structuredContent.assets[0].id;
  const preview = await call('preview_assets', { asset_ids: [assetId] }); assert.equal(preview.isError, false); assertPng(Buffer.from(preview.content.find(item => item.type === 'image').data, 'base64'));
  const scene = { version: 1, name: '中文合成', canvas: { width: 1920, height: 1080, background: '#101827' }, layers: [{ type: 'image', asset_id: assetId, x: 120, y: 80, width: 640, height: 320, fit: 'contain' }, { type: 'rect', x: 720, y: 780, width: 480, height: 80, radius: 16, color: '#2463EB' }] };
  const fonts = (await call('get_fonts')).structuredContent.fonts;
  const chineseFont = fonts.find(font => font.supports_chinese);
  if (chineseFont) scene.layers.push({ type: 'text', text: '星海中文', x: 900, y: 80, font_size: 48, font_id: chineseFont.id, color: '#FFFFFF' });
  const saved = (await call('save_design', { scene })).structuredContent; assert.equal(saved.revision, 1);
  const submitted = (await call('render_design', { design_id: saved.design_id, quality: 'final' })).structuredContent;
  const final = (await finished(submitted.job_id)).structuredContent.job;
  assert.ok(final.output_url.startsWith(`${base}/api/design-jobs/`));
  assert.equal((await fetch(final.output_url)).status, 401);
  const output = Buffer.from(await (await fetch(final.output_url, { headers: { Authorization: basic } })).arrayBuffer()); assertPng(output, 1920, 1080);
  const layout = await (await fetch(final.layout_url, { headers: { Authorization: basic } })).json(); assertSameScene(layout.scene, saved.scene);
  const changed = structuredClone(scene); changed.name = '继续修改'; changed.layers[0].x = 160;
  const second = (await call('save_design', { design_id: saved.design_id, expected_revision: 1, scene: changed })).structuredContent; assert.equal(second.revision, 2);
  assert.equal((await call('save_design', { design_id: saved.design_id, expected_revision: 1, scene })).isError, true);
  assert.equal((await call('get_design', { design_id: saved.design_id, revision: 1 })).structuredContent.scene.layers[0].x, 120);
  const previewJob = (await call('render_design', { design_id: saved.design_id, quality: 'preview' })).structuredContent;
  const scaled = await finished(previewJob.job_id); assertPng(Buffer.from(scaled.content.find(item => item.type === 'image').data, 'base64'), 1280, 720);
  assert.equal((await call('save_design', { scene: { ...scene, arbitrary_path: '/etc/passwd' } })).isError, true);

  const metadata = await (await fetch(`${base}/.well-known/oauth-protected-resource/mcp`)).json(); assert.equal(metadata.resource, `${base}/mcp`);
  const registration = await fetch(`${base}/oauth/register`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ redirect_uris: ['https://chatgpt.com/connector_platform_oauth_redirect'], client_name: 'ChatGPT smoke', token_endpoint_auth_method: 'none' }) }); assert.equal(registration.status, 201); const client = await registration.json();
  const verifier = 'smoke-pkce-verifier-012345678901234567890123456789'; const challenge = createHash('sha256').update(verifier).digest('base64url');
  const authUrl = `${base}/oauth/authorize?${new URLSearchParams({ response_type: 'code', client_id: client.client_id, redirect_uri: client.redirect_uris[0], code_challenge: challenge, code_challenge_method: 'S256', resource: `${base}/mcp`, scope: 'picsoc:read', state: 'smoke-state' })}`;
  const authPage = await (await fetch(authUrl)).text(); const requestId = authPage.match(/name="request_id" value="([^"]+)"/)[1];
  const consent = await fetch(`${base}/oauth/authorize`, { method: 'POST', redirect: 'manual', headers: { 'Content-Type': 'application/x-www-form-urlencoded', Origin: base }, body: new URLSearchParams({ request_id: requestId, password, decision: 'allow' }) }); assert.equal(consent.status, 303);
  const callback = new URL(consent.headers.get('location')); assert.equal(callback.searchParams.get('iss'), base); assert.equal(callback.searchParams.get('state'), 'smoke-state');
  const credentialsResponse = await fetch(`${base}/oauth/token`, { method: 'POST', headers: { 'Content-Type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams({ grant_type: 'authorization_code', client_id: client.client_id, redirect_uri: client.redirect_uris[0], resource: `${base}/mcp`, code: callback.searchParams.get('code'), code_verifier: verifier }) }); assert.equal(credentialsResponse.status, 200); const credentials = await credentialsResponse.json();
  assert.equal((await rpc('tools/list', {}, credentials.access_token)).status, 200);
  assert.equal((await rpc('tools/call', { name: 'render_design', arguments: { design_id: saved.design_id } }, credentials.access_token)).status, 403);
  const store = await readFile(path.join(data, 'mcp-oauth.json'), 'utf8'); assert.ok(!store.includes(credentials.access_token) && !store.includes(credentials.refresh_token));
  if (process.platform !== 'win32') assert.equal((await stat(path.join(data, 'mcp-oauth.json'))).mode & 0o777, 0o600);
  await stop(); await start(); assert.equal((await rpc('tools/list', {}, credentials.access_token)).status, 200); assert.equal((await api(`/api/designs/${saved.design_id}`)).revision, 2); assert.equal((await api(`/api/design-jobs/${submitted.job_id}`)).status, 'succeeded');

  if (process.env.PICSOC_MCP_SDK_PATH) {
    const sdk = path.resolve(process.env.PICSOC_MCP_SDK_PATH);
    const { Client } = await import(pathToFileURL(path.join(sdk, 'dist/esm/client/index.js')));
    const { StreamableHTTPClientTransport } = await import(pathToFileURL(path.join(sdk, 'dist/esm/client/streamableHttp.js')));
    const official = new Client({ name: 'Picsoc official SDK compatibility', version: '1' });
    await official.connect(new StreamableHTTPClientTransport(new URL(`${base}/mcp`), { requestInit: { headers: { Authorization: `Bearer ${token}` } } }));
    const tools = await official.listTools(); assert.equal(tools.tools.length, 10);
    const image = await official.callTool({ name: 'preview_assets', arguments: { asset_ids: [assetId] } }); assert.ok(image.content.some(item => item.type === 'image'));
    await official.close(); console.log('Official MCP SDK connection, discovery, and image tool result passed.');
  }
  console.log('MCP HTTP smoke passed: auth isolation, filtered search, actual images, PNG, editable revisions, OAuth scopes, and restart persistence.');
} catch (error) { console.error(logs); throw error; }
finally { await stop(); await rm(root, { recursive: true, force: true }); }
