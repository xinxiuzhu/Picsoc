import { readdir, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const frontend = fileURLToPath(new URL('../', import.meta.url));
const errors = [];
const pluralSuffix = /_(zero|one|two|few|many|other)$/;
function flatten(value, prefix = '', result = new Map()) {
  for (const [key, child] of Object.entries(value)) {
    const name = prefix ? `${prefix}.${key}` : key;
    if (typeof child === 'string' && child.trim()) {
      const base = name.replace(pluralSuffix, '');
      const variables = result.get(base) ?? new Set();
      for (const match of child.matchAll(/{{\s*([^{}]+?)\s*}}/g)) {
        variables.add(match[1].split(',')[0].trim());
      }
      result.set(base, variables);
    } else if (child && typeof child === 'object' && !Array.isArray(child)) {
      flatten(child, name, result);
    } else {
      errors.push(`Invalid or empty translation: ${name}`);
    }
  }
  return result;
}

const available = new Map();
for (const section of ['app', 'components', 'common']) {
  const dictionaries = await Promise.all(['zh-CN', 'en'].map(async language => {
    const source = await readFile(path.join(frontend, 'src/locales', `${section}.${language}.json`), 'utf8');
    return flatten(JSON.parse(source));
  }));
  const [chinese, english] = dictionaries;
  for (const key of new Set([...chinese.keys(), ...english.keys()])) {
    if (!chinese.has(key) || !english.has(key)) {
      errors.push(`Missing ${!chinese.has(key) ? 'zh-CN' : 'en'} translation: ${key}`);
      continue;
    }
    const left = [...chinese.get(key)].sort().join(',');
    const right = [...english.get(key)].sort().join(',');
    if (left !== right) errors.push(`Interpolation mismatch for ${key}: ${left} / ${right}`);
    available.set(key, true);
  }
}

async function checkReferences(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const sourcePath = path.join(directory, entry.name);
    if (entry.isDirectory()) { await checkReferences(sourcePath); continue; }
    if (!/\.tsx?$/.test(entry.name)) continue;
    const source = await readFile(sourcePath, 'utf8');
    for (const match of source.matchAll(/\bt\(\s*['"]([^'"]+)['"]/g)) {
      if (!available.has(match[1].replace(pluralSuffix, ''))) {
        errors.push(`Unknown translation key ${match[1]} in ${path.relative(frontend, sourcePath)}`);
      }
    }
  }
}
await checkReferences(path.join(frontend, 'src'));
if (errors.length) {
  process.stderr.write(errors.join('\n') + '\n');
  process.exitCode = 1;
} else {
  process.stdout.write(`Checked ${available.size} translation keys in zh-CN and en.\n`);
}
