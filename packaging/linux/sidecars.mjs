// The programs the Linux server package carries beside lull (ADR-0090): the slskd, yt-dlp and Deno that
// app/build.rs pins for the desktop app's Linux build, read from it (one place for each pin), downloaded,
// checked by sha256 and unpacked into <dir> as slskd, yt-dlp and deno, with slskd's licence.
//
//   node packaging/linux/sidecars.mjs <rust target> <dir>
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { chmod, mkdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const [target, dir] = process.argv.slice(2);
if (!target || !dir) throw new Error('usage: sidecars.mjs <rust target> <dir>');
const buildRs = await readFile(new URL('../../app/build.rs', import.meta.url), 'utf8');

const constant = (name) => {
  const m = buildRs.match(new RegExp(`const ${name}: &str = "([^"]+)";`));
  if (!m) throw new Error(`app/build.rs has no ${name}`);
  return m[1];
};
// Each `Sidecar { … }` / `Tool { … }` literal, as a map of its fields.
const blocks = (kind) =>
  [...buildRs.matchAll(new RegExp(`${kind} \\{([^}]*)\\}`, 'g'))].map(([, body]) =>
    Object.fromEntries([...body.matchAll(/(\w+): (?:Some\()?"([^"]*)"/g)].map(([, k, v]) => [k, v])),
  );
const pinned = (list, what) => {
  const found = list.filter((b) => b.target === target);
  if (found.length === 0) throw new Error(`app/build.rs pins no ${what} for ${target}`);
  return found;
};

const [slskd] = pinned(blocks('Sidecar'), 'slskd');
const versions = { 'yt-dlp': constant('YTDLP_VERSION'), deno: constant('DENO_VERSION') };
const tools = pinned(blocks('Tool'), 'yt-dlp or Deno');
await mkdir(dir, { recursive: true });

async function fetchChecked(url, sha256) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url}: ${res.status}`);
  const bytes = Buffer.from(await res.arrayBuffer());
  const got = createHash('sha256').update(bytes).digest('hex');
  if (got !== sha256) throw new Error(`${url} has sha256 ${got}, not the pinned ${sha256}`);
  return bytes;
}
async function unzip(bytes, entry, to) {
  const zip = join(dir, '.download.zip');
  await writeFile(zip, bytes);
  await writeFile(to, execFileSync('unzip', ['-p', zip, entry], { maxBuffer: 1 << 30 }));
  execFileSync('rm', ['-f', zip]);
}

const slskdZip = await fetchChecked(slskd.url, slskd.sha256);
await unzip(slskdZip, 'slskd', join(dir, 'slskd'));
await unzip(slskdZip, 'LICENSE', join(dir, 'slskd-LICENSE.txt'));
for (const tool of tools) {
  const url =
    tool.name === 'deno'
      ? `https://github.com/denoland/deno/releases/download/v${versions.deno}/${tool.asset}`
      : `https://github.com/yt-dlp/yt-dlp/releases/download/${versions['yt-dlp']}/${tool.asset}`;
  const bytes = await fetchChecked(url, tool.sha256);
  const to = join(dir, tool.name);
  if (tool.zipped) await unzip(bytes, tool.zipped, to);
  else await writeFile(to, bytes);
}
for (const name of ['slskd', 'yt-dlp', 'deno']) await chmod(join(dir, name), 0o755);
console.log(`slskd, yt-dlp ${versions['yt-dlp']} and Deno ${versions.deno} for ${target} in ${dir}`);
