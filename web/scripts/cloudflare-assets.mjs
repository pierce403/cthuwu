import { mkdirSync, writeFileSync, appendFileSync, readdirSync, readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';
const output = resolve("dist");
const revision = process.env.CF_COMMIT_SHA || execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
if (!/^[a-f0-9]{40}$/.test(revision)) throw new Error('Invalid source revision');
mkdirSync(resolve(output, '.well-known'), { recursive: true });
writeFileSync(resolve(output, '.well-known/cloudflare-build.json'), JSON.stringify({ repository: "pierce403/cthuwu", revision }, null, 2) + '\n');
writeFileSync(resolve(output, '404.html'), '<!doctype html><html lang="en"><meta charset="utf-8"><title>Page not found</title><h1>Page not found</h1><p><a href="/">Return home</a></p></html>\n');
appendFileSync(resolve(output, '_headers'), '\n/*\n  Cache-Control: public, max-age=0, must-revalidate, no-transform\n/.well-known/cloudflare-build.json\n  Cache-Control: no-store\n');
console.log('Cloudflare static assets prepared at ' + revision);

const files = [];
function inventory(directory, prefix = '') {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = prefix + entry.name;
    if (entry.isDirectory()) inventory(resolve(directory, entry.name), path + '/');
    else if (!['_headers', '404.html', '.well-known/cloudflare-build.json', '.well-known/cloudflare-assets.json'].includes(path)) {
      const bytes = readFileSync(resolve(directory, entry.name));
      files.push({ path, bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') });
    }
  }
}
inventory(output);
writeFileSync(resolve(output, '.well-known/cloudflare-assets.json'), JSON.stringify({ repository: 'pierce403/cthuwu', revision, files }, null, 2) + '\n');
appendFileSync(resolve(output, '_headers'), '\n/.well-known/cloudflare-assets.json\n  Cache-Control: no-store\n');
