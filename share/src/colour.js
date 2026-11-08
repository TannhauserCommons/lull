// A cover's colour, for the edge of lull's Discord card (docs/adr/0034, "The Discord card"): worked out once, by the
// cover's lookup in its Durable Object, from a small copy of the picture; only the colour is kept.
//
// Covers come as JPEG (Apple, Deezer, the Cover Art Archive, Wikimedia) or PNG (Last.fm's):
// - A JPEG stores each 8×8 block's average as its first ("DC") coefficient, so the average colours of a small cover
//   come from its first scan without the full decode (no inverse DCT, no upsampling): a 128 px cover gives an 8×8 to
//   16×16 grid of colours in a millisecond or two. Baseline and progressive JPEGs are read.
// - A PNG is inflated (the platform's DecompressionStream) and unfiltered, and averaged over 8×8 blocks.
// Anything else (arithmetic-coded or CMYK JPEGs, interlaced PNGs, other formats) has no colour, and
// the card keeps lull's own.

/** Which reader a kept colour came from: a colour read by an older one (or none it could read) is read again. */
export const READER = 2;

/** The colour of a cover, `#rrggbb`, from its JPEG or PNG bytes; null when it isn't a picture this reads. */
export async function coverColour(bytes) {
  const data = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let grid;
  try {
    grid = isPng(data) ? await pngAverages(data) : averages(data);
  } catch (e) {
    if (e instanceof Unreadable) return null;
    throw e;
  }
  return grid && grid.length ? hex(accent(grid)) : null;
}

class Unreadable extends Error {}
function unreadable(why) { return new Unreadable(why); }

// ---- The averages in a JPEG ----

/** Each MCU's average colour, as [r, g, b] 0–255, read from the DC coefficients of the picture's scans. */
function averages(data) {
  if (data[0] !== 0xff || data[1] !== 0xd8) throw unreadable('not a JPEG');
  const quant = [];
  const dc = [];
  const ac = [];
  let frame = null;
  let restart = 0;
  let adobe = null;
  let at = 2;
  while (at < data.length) {
    if (data[at] !== 0xff) throw unreadable('lost the markers');
    const marker = data[at + 1];
    at += 2;
    if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7) || marker === 0xff) {
      if (marker === 0xff) at -= 1;
      continue;
    }
    if (marker === 0xd9) break;
    const length = (data[at] << 8) | data[at + 1];
    const body = data.subarray(at + 2, at + length);
    at += length;
    if (marker === 0xdb) readQuant(body, quant);
    else if (marker === 0xc4) readHuffman(body, dc, ac);
    else if (marker === 0xdd) restart = (body[0] << 8) | body[1];
    else if (marker === 0xee && body.length >= 12 && String.fromCharCode(...body.subarray(0, 5)) === 'Adobe') adobe = body[11];
    else if (marker === 0xc0 || marker === 0xc1 || marker === 0xc2) frame = readFrame(body, marker === 0xc2);
    else if (marker >= 0xc3 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) {
      throw unreadable('a JPEG coding this doesn\'t read');
    } else if (marker === 0xda) {
      if (!frame) throw unreadable('a scan before its frame');
      at = readScan(data, at, body, frame, quant, dc, ac, restart);
      if (frame.components.every((c) => c.done)) return colours(frame, adobe);
    }
  }
  throw unreadable('the picture ended before its averages');
}

function readQuant(body, quant) {
  for (let i = 0; i < body.length;) {
    const wide = body[i] >> 4;
    const id = body[i] & 15;
    // Only the first value, the DC coefficient's step, is needed.
    quant[id] = wide ? (body[i + 1] << 8) | body[i + 2] : body[i + 1];
    i += 1 + 64 * (wide ? 2 : 1);
  }
}

/** Huffman tables as maps from (length, code) to value, the canonical way JPEG assigns codes. */
function readHuffman(body, dc, ac) {
  for (let i = 0; i < body.length;) {
    const cls = body[i] >> 4;
    const id = body[i] & 15;
    const counts = body.subarray(i + 1, i + 17);
    let v = i + 17;
    const table = new Map();
    let code = 0;
    for (let len = 1; len <= 16; len++) {
      for (let n = 0; n < counts[len - 1]; n++) table.set(len * 65536 + code++, body[v++]);
      code <<= 1;
    }
    (cls ? ac : dc)[id] = table;
    i = v;
  }
}

function readFrame(body, progressive) {
  const height = (body[1] << 8) | body[2];
  const width = (body[3] << 8) | body[4];
  const count = body[5];
  if (!width || !height || (count !== 1 && count !== 3)) throw unreadable('not a greyscale or three-colour picture');
  const components = [];
  for (let i = 0; i < count; i++) {
    const o = 6 + i * 3;
    components.push({ id: body[o], h: body[o + 1] >> 4, v: body[o + 1] & 15, table: body[o + 2], step: 0, done: false });
  }
  const hMax = Math.max(...components.map((c) => c.h));
  const vMax = Math.max(...components.map((c) => c.v));
  const mcusX = Math.ceil(width / (8 * hMax));
  const mcusY = Math.ceil(height / (8 * vMax));
  for (const c of components) {
    c.across = mcusX * c.h;
    c.blocks = new Int32Array(mcusX * c.h * mcusY * c.v);
    // A scan of this component alone covers only the blocks inside the picture, not the MCUs' padding.
    c.ownX = Math.ceil(Math.ceil((width * c.h) / hMax) / 8);
    c.ownY = Math.ceil(Math.ceil((height * c.v) / vMax) / 8);
  }
  return { progressive, components, mcusX, mcusY };
}

/**
 * One scan: the DC coefficients of the components it carries, when it is a baseline scan or a progressive scan's
 * first DC pass; any other scan is passed over. Answers where the entropy-coded data ends.
 */
function readScan(data, at, header, frame, quant, dc, ac, restart) {
  const n = header[0];
  const comps = [];
  for (let i = 0; i < n; i++) {
    const c = frame.components.find((x) => x.id === header[1 + i * 2]);
    if (!c) throw unreadable('a scan of an unknown component');
    comps.push({ c, dc: dc[header[2 + i * 2] >> 4], ac: ac[header[2 + i * 2] & 15], pred: 0 });
  }
  const ss = header[1 + n * 2];
  const se = header[2 + n * 2];
  const ah = header[3 + n * 2] >> 4;
  const al = header[3 + n * 2] & 15;
  const firstDc = ss === 0 && ah === 0 && (frame.progressive ? se === 0 : se === 63);
  if (!firstDc) return skipEntropy(data, at);
  if (comps.some((s) => !s.dc || (!frame.progressive && !s.ac))) throw unreadable('a scan with a missing Huffman table');
  // A component's quantisation table may come after the frame; the one in force is the one at its scan.
  for (const { c } of comps) {
    c.step = quant[c.table];
    if (!c.step) throw unreadable('a component with no quantisation table');
  }

  const bits = reader(data, at);
  const single = n === 1;
  const units = single ? comps[0].c.ownX * comps[0].c.ownY : frame.mcusX * frame.mcusY;
  for (let u = 0; u < units; u++) {
    if (restart && u > 0 && u % restart === 0) {
      bits.restart();
      for (const s of comps) s.pred = 0;
    }
    for (const s of comps) {
      const { c } = s;
      const blocksHere = single ? [[u % c.ownX, Math.floor(u / c.ownX)]] : blocksOfMcu(c, u % frame.mcusX, Math.floor(u / frame.mcusX));
      for (const [bx, by] of blocksHere) {
        const size = bits.decode(s.dc);
        s.pred += size ? extend(bits.take(size), size) : 0;
        c.blocks[by * c.across + bx] = s.pred * (1 << al);
        if (!frame.progressive) skipAc(bits, s.ac);
      }
    }
  }
  for (const s of comps) s.c.done = true;
  return skipEntropy(data, bits.position());
}

function blocksOfMcu(c, mx, my) {
  const out = [];
  for (let y = 0; y < c.v; y++) for (let x = 0; x < c.h; x++) out.push([mx * c.h + x, my * c.v + y]);
  return out;
}

function skipAc(bits, table) {
  for (let k = 1; k < 64;) {
    const rs = bits.decode(table);
    const size = rs & 15;
    if (!size) {
      if ((rs >> 4) !== 15) return; // end of block
      k += 16;
      continue;
    }
    bits.take(size);
    k += (rs >> 4) + 1;
  }
}

function extend(value, size) {
  return value < 1 << (size - 1) ? value - (1 << size) + 1 : value;
}

/** Past the entropy-coded data that starts at `at`: to the next marker that isn't a restart. */
function skipEntropy(data, at) {
  for (let i = at; i < data.length - 1; i++) {
    if (data[i] === 0xff && data[i + 1] !== 0 && !(data[i + 1] >= 0xd0 && data[i + 1] <= 0xd7)) return i;
  }
  return data.length;
}

/** Reads the entropy-coded bits, undoing JPEG's byte stuffing (0xFF 0x00); stops at a marker. */
function reader(data, start) {
  let at = start;
  let byte = 0;
  let left = 0;
  const bit = () => {
    if (!left) {
      if (at >= data.length) throw unreadable('the scan ended early');
      byte = data[at++];
      if (byte === 0xff) {
        const next = data[at];
        if (next === 0) at++;
        else throw unreadable('a marker inside a scan');
      }
      left = 8;
    }
    left--;
    return (byte >> left) & 1;
  };
  return {
    take(n) {
      let v = 0;
      for (let i = 0; i < n; i++) v = (v << 1) | bit();
      return v;
    },
    decode(table) {
      let code = 0;
      for (let len = 1; len <= 16; len++) {
        code = (code << 1) | bit();
        const v = table.get(len * 65536 + code);
        if (v !== undefined) return v;
      }
      throw unreadable('a code no table has');
    },
    /** At a restart interval: drop the byte's leftover bits and step over the RSTn marker. */
    restart() {
      left = 0;
      if (data[at] === 0xff && data[at + 1] >= 0xd0 && data[at + 1] <= 0xd7) at += 2;
      else throw unreadable('a missing restart marker');
    },
    position: () => at,
  };
}

/** One colour per MCU: each component's blocks in it averaged, then turned into RGB. */
function colours(frame, adobe) {
  const { components, mcusX, mcusY } = frame;
  const out = [];
  for (let my = 0; my < mcusY; my++) {
    for (let mx = 0; mx < mcusX; mx++) {
      const values = components.map((c) => {
        let sum = 0;
        for (const [bx, by] of blocksOfMcu(c, mx, my)) sum += c.blocks[by * c.across + bx];
        // A block's average sample is its dequantised DC coefficient over 8, around 128.
        return (sum / (c.h * c.v)) * c.step / 8 + 128;
      });
      if (values.length === 1) out.push([values[0], values[0], values[0]].map(clamp255));
      // Adobe's transform 0 marks a three-component JPEG stored as RGB, not YCbCr.
      else if (adobe === 0) out.push(values.map(clamp255));
      else {
        const [y, cb, cr] = values;
        out.push([y + 1.402 * (cr - 128), y - 0.344136 * (cb - 128) - 0.714136 * (cr - 128), y + 1.772 * (cb - 128)].map(clamp255));
      }
    }
  }
  return out;
}

function clamp255(v) { return Math.min(255, Math.max(0, v)); }

// ---- The averages in a PNG ----

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
/** Channels per PNG colour type: grey, RGB, palette, grey and alpha, RGBA. */
const PNG_CHANNELS = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 };
/** A small copy is at most a few hundred pixels a side; anything far larger isn't inflated. */
const PNG_MAX_PIXELS = 1024 * 1024;

function isPng(data) {
  return PNG_SIGNATURE.every((b, i) => data[i] === b);
}

/** Each 8×8 block's average colour, as [r, g, b] 0–255, weighted by its pixels' opacity; clear blocks are left out. */
async function pngAverages(data) {
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  let header = null;
  let palette = null;
  let clear = null;
  const idat = [];
  for (let at = 8; at + 8 <= data.length;) {
    const length = view.getUint32(at);
    const type = String.fromCharCode(...data.subarray(at + 4, at + 8));
    const body = data.subarray(at + 8, at + 8 + length);
    if (body.length !== length) throw unreadable('the PNG ended inside a chunk');
    if (type === 'IHDR') {
      header = { width: view.getUint32(at + 8), height: view.getUint32(at + 12), depth: body[8], type: body[9], interlace: body[12] };
    } else if (type === 'PLTE') palette = body;
    else if (type === 'tRNS') clear = body;
    else if (type === 'IDAT') idat.push(body);
    else if (type === 'IEND') break;
    at += 12 + length;
  }
  if (!header || !idat.length) throw unreadable('a PNG with no picture');
  const { width, height, depth, type, interlace } = header;
  const channels = PNG_CHANNELS[type];
  // Every depth PNG allows: 1, 2, 4 and 8 bits for grey and palettes, 8 and 16 for the rest.
  const depths = type === 0 ? [1, 2, 4, 8, 16] : type === 3 ? [1, 2, 4, 8] : [8, 16];
  if (!channels || interlace || !depths.includes(depth) || (type === 3 && !palette)) {
    throw unreadable('a PNG kind this doesn\'t read');
  }
  if (!width || !height || width * height > PNG_MAX_PIXELS) throw unreadable('a PNG of no size, or too large');
  const pixels = await inflate(idat);
  const stride = Math.ceil((width * channels * depth) / 8);
  const bpp = Math.max(1, (channels * depth) / 8);
  if (pixels.length < height * (stride + 1)) throw unreadable('a PNG shorter than its size');
  const rows = unfilter(pixels, height, stride, bpp);
  const sample = sampler(depth, channels);

  const across = Math.ceil(width / 8);
  const sums = new Float64Array(across * Math.ceil(height / 8) * 4);
  for (let y = 0; y < height; y++) {
    const row = rows[y];
    for (let x = 0; x < width; x++) {
      const [r, g, b, a] = pixel((i) => sample(row, x, i), type, palette, clear, depth);
      const k = ((y >> 3) * across + (x >> 3)) * 4;
      sums[k] += r * a;
      sums[k + 1] += g * a;
      sums[k + 2] += b * a;
      sums[k + 3] += a;
    }
  }
  const out = [];
  for (let k = 0; k < sums.length; k += 4) if (sums[k + 3] > 0) out.push([sums[k], sums[k + 1], sums[k + 2]].map((v) => v / sums[k + 3]));
  return out;
}

/**
 * Reads channel `i` of pixel `x` in a row: 16-bit samples by their high byte, samples under 8 bits unpacked (a
 * palette index as it is, a grey scaled to 0–255).
 */
function sampler(depth, channels) {
  if (depth >= 8) {
    const step = depth / 8;
    return (row, x, i) => row[(x * channels + i) * step];
  }
  const mask = (1 << depth) - 1;
  return (row, x) => (row[(x * depth) >> 3] >> (8 - depth - ((x * depth) & 7))) & mask;
}

/** One pixel as [r, g, b, alpha 0–1], from `s(i)`, its channel i. */
function pixel(s, type, palette, clear, depth = 8) {
  if (type === 3) {
    const i = s(0) * 3;
    return [palette[i], palette[i + 1], palette[i + 2], clear && s(0) < clear.length ? clear[s(0)] / 255 : 1];
  }
  if (type === 0) {
    const v = depth < 8 ? (s(0) * 255) / ((1 << depth) - 1) : s(0);
    return [v, v, v, 1];
  }
  if (type === 4) return [s(0), s(0), s(0), s(1) / 255];
  if (type === 2) return [s(0), s(1), s(2), 1];
  return [s(0), s(1), s(2), s(3) / 255];
}

async function inflate(chunks) {
  try {
    const stream = new Blob(chunks).stream().pipeThrough(new DecompressionStream('deflate'));
    return new Uint8Array(await new Response(stream).arrayBuffer());
  } catch {
    throw unreadable('a PNG whose picture doesn\'t inflate');
  }
}

/** PNG's five row filters undone: each row's bytes as they were before filtering. */
function unfilter(data, height, stride, bpp) {
  const rows = [];
  let prev = new Uint8Array(stride);
  for (let y = 0; y < height; y++) {
    const at = y * (stride + 1);
    const filter = data[at];
    const row = data.slice(at + 1, at + 1 + stride);
    for (let i = 0; i < stride; i++) {
      const left = i >= bpp ? row[i - bpp] : 0;
      const up = prev[i];
      const corner = i >= bpp ? prev[i - bpp] : 0;
      if (filter === 1) row[i] += left;
      else if (filter === 2) row[i] += up;
      else if (filter === 3) row[i] += (left + up) >> 1;
      else if (filter === 4) row[i] += paeth(left, up, corner);
      else if (filter !== 0) throw unreadable('a PNG row with an unknown filter');
    }
    rows.push(row);
    prev = row;
  }
  return rows;
}

function paeth(a, b, c) {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}

// ---- From the averages to one colour ----

/** Below this mean chroma (OKLCH) a cover reads as black and white, and its card edge stays grey. */
const GREY = 0.03;
/** The edge is drawn on Discord's dark and light themes alike: a lightness and chroma that read on both. */
const L_MIN = 0.5;
const L_MAX = 0.72;
const C_MAX = 0.19;
const HUES = 24;

/**
 * The cover's strongest colour, as Spotify's cards take one: the hue that the most colourful area of the cover
 * shares, weighted by how colourful each part is, then brought to a lightness that reads on any theme.
 */
function accent(rgbs) {
  const labs = rgbs.map(oklab);
  const chroma = (p) => Math.hypot(p[1], p[2]);
  const meanChroma = labs.reduce((s, p) => s + chroma(p), 0) / labs.length;
  let pick;
  if (meanChroma < GREY) {
    pick = mean(labs, () => 1);
  } else {
    const bins = new Float64Array(HUES);
    const binOf = (p) => Math.floor(((Math.atan2(p[2], p[1]) / (2 * Math.PI) + 1) % 1) * HUES) % HUES;
    for (const p of labs) bins[binOf(p)] += chroma(p);
    let best = 0;
    for (let b = 1; b < HUES; b++) if (bins[b] > bins[best]) best = b;
    // The winning hue and its neighbours, so a colour split across a bin's edge isn't halved.
    const near = (p) => Math.min((binOf(p) - best + HUES) % HUES, (best - binOf(p) + HUES) % HUES) <= 1;
    pick = mean(labs.filter(near), (p) => chroma(p) ** 2);
  }
  return fit(pick);
}

function mean(points, weight) {
  let w = 0;
  const sum = [0, 0, 0];
  for (const p of points) {
    const k = weight(p);
    w += k;
    for (let i = 0; i < 3; i++) sum[i] += p[i] * k;
  }
  return sum.map((s) => s / w);
}

/** The colour at a lightness the edge reads at, its chroma capped and then lowered until it is in sRGB. */
function fit([l, a, b]) {
  const L = Math.min(L_MAX, Math.max(L_MIN, l));
  const c = Math.hypot(a, b);
  const h = Math.atan2(b, a);
  for (let C = Math.min(c, C_MAX); ; C -= 0.005) {
    const rgb = srgb([L, C * Math.cos(h), C * Math.sin(h)]);
    if (rgb || C <= 0) return rgb || srgb([L, 0, 0]);
  }
}

// OKLab (Björn Ottosson's), from and to sRGB 0–255.
function oklab([r, g, b]) {
  const [R, G, B] = [r, g, b].map((v) => {
    v /= 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  const l = Math.cbrt(0.4122214708 * R + 0.5363325363 * G + 0.0514459929 * B);
  const m = Math.cbrt(0.2119034982 * R + 0.6806995451 * G + 0.1073969566 * B);
  const s = Math.cbrt(0.0883024619 * R + 0.2817188376 * G + 0.6299787005 * B);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

/** sRGB 0–255 for an OKLab colour, or null when it falls outside sRGB. */
function srgb([L, a, b]) {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const lin = [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
  if (lin.some((v) => v < -1e-4 || v > 1 + 1e-4)) return null;
  return lin.map((v) => {
    v = Math.min(1, Math.max(0, v));
    return Math.round(255 * (v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055));
  });
}

function hex(rgb) {
  return '#' + rgb.map((v) => v.toString(16).padStart(2, '0')).join('');
}
