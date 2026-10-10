// Test startup configuration through the real Rust process, without Node dependencies.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, readFile, writeFile, stat, rm, realpath } from 'node:fs/promises';
import net from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';

const binary = path.resolve(process.argv[2] || 'target/debug/picsoc');
const root = await realpath(await mkdtemp(path.join(tmpdir(), 'picsoc-config-')));
const data = path.join(root, 'data');
const configPath = path.join(data, 'config.toml');
const password = '星海"引号\\路径秘密';
const obsoletePassword = 'obsolete-env-secret-must-not-work';
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('PICSOC_')));
Object.assign(env, {
  PICSOC_BIND: 'invalid-obsolete-bind',
  PICSOC_DATA_DIR: path.join(root, 'obsolete-data'),
  PICSOC_PASSWORD: obsoletePassword,
  PICSOC_WORKERS: 'invalid-obsolete-workers',
  PICSOC_SCAN_INTERVAL: 'invalid-obsolete-interval',
  PICSOC_MCP_ENABLED: 'invalid-obsolete-enabled',
  PICSOC_PUBLIC_URL: 'invalid-obsolete-public-url',
  PICSOC_MCP_TOKEN: 'too-short',
  PICSOC_MCP_REDIRECT_URIS: 'invalid-obsolete-redirect',
});
let active;

async function freePort() {
  const socket = net.createServer();
  socket.listen(0, '127.0.0.1');
  await once(socket, 'listening');
  const port = socket.address().port;
  await new Promise(resolve => socket.close(resolve));
  return port;
}

function launch(args) {
  const child = spawn(binary, args, { env, stdio: ['ignore', 'pipe', 'pipe'] });
  const running = { child, logs: '', done: undefined, spawnError: undefined };
  child.stdout.on('data', bytes => { running.logs += bytes; });
  child.stderr.on('data', bytes => { running.logs += bytes; });
  running.done = new Promise(resolve => {
    child.once('error', error => { running.spawnError = error; resolve(undefined); });
    child.once('close', code => resolve(code));
  });
  active = running;
  return running;
}

async function ready(running, port) {
  const base = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 150; i++) {
    if (running.spawnError) throw running.spawnError;
    if (running.child.exitCode !== null || running.child.signalCode !== null) {
      throw new Error(`Service stopped before listening: ${running.logs}`);
    }
    try {
      const response = await fetch(`${base}/api/auth/status`, { signal: AbortSignal.timeout(1000) });
      if (response.ok) return { base, auth: await response.json() };
    } catch {}
    await wait(100);
  }
  throw new Error(`Service did not start: ${running.logs}`);
}

async function stop(running = active) {
  if (!running) return;
  const timer = setTimeout(() => {
    if (running.child.exitCode === null && running.child.signalCode === null) running.child.kill('SIGKILL');
  }, 10000);
  try {
    if (running.child.exitCode === null && running.child.signalCode === null) running.child.kill('SIGINT');
    await running.done;
  } finally {
    clearTimeout(timer);
    if (active === running) active = undefined;
  }
}

async function completed(args) {
  const running = launch(args);
  const timer = setTimeout(() => {
    if (running.child.exitCode === null && running.child.signalCode === null) running.child.kill('SIGKILL');
  }, 10000);
  try {
    const code = await running.done;
    if (running.spawnError) throw running.spawnError;
    assert.equal(running.child.signalCode, null, `Command did not exit promptly: ${running.logs}`);
    return { code, logs: running.logs };
  } finally {
    clearTimeout(timer);
    if (active === running) active = undefined;
  }
}

function configuration(port, dataDir, secret = password) {
  // JSON basic-string escapes are TOML-compatible for these paths and test passwords.
  return [
    '# 用户编辑：保留此注释和文件内容',
    `bind = ${JSON.stringify(`127.0.0.1:${port}`)}`,
    `data_dir = ${JSON.stringify(dataDir)}`,
    'open_browser = false',
    'workers = 1',
    'scan_interval = 0',
    `password = ${JSON.stringify(secret)}`,
    '',
    '[mcp]',
    'enabled = false',
    'public_url = ""',
    'token = ""',
    'redirect_uris = []',
    '',
  ].join('\n');
}

async function assertPassword(base) {
  const login = secret => fetch(`${base}/api/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ password: secret }),
  });
  assert.equal((await login(obsoletePassword)).status, 401);
  assert.equal((await login(password)).status, 200);
}

function assertNoSecrets(logs) {
  for (const secret of [password, obsoletePassword]) {
    assert.ok(!logs.includes(secret) && !logs.includes(JSON.stringify(secret).slice(1, -1)), 'Logs must not expose passwords');
  }
}

async function assertNoFile(filename) {
  await assert.rejects(stat(filename), error => error.code === 'ENOENT');
}

try {
  // Informational invocations must not create directories or configuration files.
  for (const flag of ['--help', '--version']) {
    const infoData = path.join(root, flag.slice(2));
    const result = await completed(['--data-dir', infoData, flag]);
    assert.equal(result.code, 0, result.logs);
    await assertNoFile(infoData);
  }

  // First launch writes an editable template while continuing to serve HTTP.
  const firstPort = await freePort();
  const first = launch(['--data-dir', data, '--bind', `127.0.0.1:${firstPort}`, '--no-open', '--scan-interval', '0']);
  const firstState = await ready(first, firstPort);
  assert.equal(firstState.auth.password_required, false);
  const generated = await readFile(configPath, 'utf8');
  assert.ok(generated.includes('[mcp]') && generated.includes('open_browser'));
  assert.ok(first.logs.includes(configPath), `Startup must show its configuration file: ${first.logs}`);
  assertNoSecrets(first.logs);
  if (process.platform !== 'win32') assert.equal((await stat(configPath)).mode & 0o777, 0o600);
  await stop(first);

  // The user's file changes take effect with only --data-dir and remain byte-for-byte intact.
  const configuredPort = await freePort();
  const edited = configuration(configuredPort, data);
  await writeFile(configPath, edited, 'utf8');
  for (let restart = 0; restart < 2; restart++) {
    const running = launch(['--data-dir', data]);
    const state = await ready(running, configuredPort);
    assert.equal(state.auth.password_required, true);
    await assertPassword(state.base);
    assert.equal((await fetch(`${state.base}/mcp`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}',
    })).status, 404);
    assert.ok(running.logs.includes(configPath));
    assertNoSecrets(running.logs);
    assert.equal(await readFile(configPath, 'utf8'), edited);
    await stop(running);
  }
  await assertNoFile(path.join(root, 'obsolete-data'));

  // Reject malformed or invalid configuration without rewriting it or opening HTTP.
  const invalidPort = await freePort();
  for (const invalid of [
    `bind = "127.0.0.1:${invalidPort}"\npassword = "unfinished\n`,
    configuration(invalidPort, data).replace('workers = 1', 'workers = 0'),
  ]) {
    await writeFile(configPath, invalid, 'utf8');
    const result = await completed(['--data-dir', data]);
    assert.notEqual(result.code, 0, result.logs);
    assert.equal(await readFile(configPath, 'utf8'), invalid);
    assertNoSecrets(result.logs);
    await assert.rejects(fetch(`http://127.0.0.1:${invalidPort}/api/auth/status`, { signal: AbortSignal.timeout(1000) }));
  }

  // A custom file resolves relative data_dir against its directory, across platforms.
  const customParent = path.join(root, '自定义配置');
  await mkdir(customParent);
  const customPath = path.join(customParent, 'settings.toml');
  const customPort = await freePort();
  const custom = configuration(customPort, 'relative-data');
  await writeFile(customPath, custom, { mode: 0o600 });
  const customRunning = launch(['--config', customPath]);
  const customState = await ready(customRunning, customPort);
  await assertPassword(customState.base);
  assert.ok(customRunning.logs.includes(customPath));
  assert.equal(await readFile(customPath, 'utf8'), custom);
  assert.ok((await stat(path.join(customParent, 'relative-data', 'picsoc.sqlite3'))).isFile());
  assertNoSecrets(customRunning.logs);
  await stop(customRunning);

  console.log('Configuration smoke passed: first-run TOML, Ctrl+C, edited settings, password escaping, ignored legacy environment, restart preservation, invalid files, and custom relative paths.');
} catch (error) {
  if (active) console.error(active.logs);
  throw error;
} finally {
  await stop();
  await rm(root, { recursive: true, force: true });
}
