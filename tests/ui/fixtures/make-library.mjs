// A small fixture music library for the visual tests: `node fixtures/make-library.mjs <out dir>`.
// ffmpeg writes a short sine tone per track, tagged from library.json and carrying a generated
// cover, laid out as lull saves files (Artist/Album (Year)/NN - Title.ext). It is the folder
// `LULL_MUSIC_DIR` points at (ADR-0003). Needs ffmpeg on PATH; never commit its output.

import { execFile } from 'node:child_process';
import { mkdir, readFile, rm } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const run = promisify(execFile);
const here = dirname(fileURLToPath(import.meta.url));
const out = process.argv[2] && resolve(process.argv[2]);
if (!out) {
  console.error('usage: node make-library.mjs <out dir>');
  process.exit(2);
}

const { albums } = JSON.parse(await readFile(join(here, 'library.json'), 'utf8'));
const SECONDS = 6;
const BASE_HZ = 220;

await rm(out, { recursive: true, force: true });
let tracks = 0;
for (const [a, album] of albums.entries()) {
  const dir = join(out, safe(album.artist), `${safe(album.album)} (${album.year})`);
  await mkdir(dir, { recursive: true });
  const cover = join(dir, 'cover.jpg');
  await ffmpeg([
    '-f', 'lavfi', '-i', `gradients=s=600x600:c0=${hex(album.cover[0])}:c1=${hex(album.cover[1])}:seed=${a + 1}:d=1`,
    '-frames:v', '1', '-q:v', '3', cover,
  ]);
  for (const [t, title] of album.tracks.entries()) {
    const no = t + 1;
    const file = join(dir, `${String(no).padStart(2, '0')} - ${safe(title)}.${album.format}`);
    // A different pitch per track, so the files differ and a stream is audibly a stream.
    const hz = BASE_HZ + (a * 4 + t) * 22;
    await ffmpeg([
      '-f', 'lavfi', '-i', `sine=frequency=${hz}:duration=${SECONDS}:sample_rate=44100`,
      '-i', cover,
      '-map', '0:a', '-map', '1:v',
      ...(album.format === 'flac' ? ['-c:a', 'flac'] : ['-c:a', 'libmp3lame', '-b:a', '128k', '-id3v2_version', '3']),
      '-c:v', 'mjpeg', '-disposition:v', 'attached_pic',
      '-metadata', `title=${title}`,
      '-metadata', `artist=${album.artist}`,
      '-metadata', `album_artist=${album.artist}`,
      '-metadata', `album=${album.album}`,
      '-metadata', `date=${album.year}`,
      '-metadata', `genre=${album.genre}`,
      '-metadata', `track=${no}/${album.tracks.length}`,
      '-metadata', 'disc=1/1',
      file,
    ]);
    tracks += 1;
  }
}
console.log(`fixture library: ${albums.length} albums, ${tracks} tracks in ${out}`);

// bitexact keeps the bytes the same on every run, so the same library gives the same ids and sizes.
function ffmpeg(args) {
  return run('ffmpeg', ['-v', 'error', '-y', '-fflags', '+bitexact', '-flags:a', '+bitexact', ...args]);
}
function hex(color) {
  return `0x${color.slice(1)}`;
}
function safe(name) {
  return name.replace(/[\\/:*?"<>|]/g, '_');
}
