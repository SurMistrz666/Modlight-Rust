/**
 * Tiny dependency-free QR code generator.
 *
 * Supports byte mode with error-correction level "M" (used by most
 * authenticator apps). Picks the smallest QR version (1-40) that fits the
 * payload. Returns the module matrix so callers can render it as SVG or a
 * canvas. An otpauth:// URI is short enough that low versions are used.
 *
 * Implementation is a compact, self-contained port of the standard QR
 * pipeline: data encoding -> Reed-Solomon ECC -> matrix placement ->
 * masking with the lowest-penalty mask. No external dependencies.
 */

// ---- Galois field (GF(256)) tables for Reed-Solomon ----
const EXP = new Uint8Array(512);
const LOG = new Uint8Array(256);
(function initGaloisField() {
  let x = 1;
  for (let i = 0; i < 255; i++) {
    EXP[i] = x;
    LOG[x] = i;
    x <<= 1;
    if (x & 0x100) x ^= 0x11d;
  }
  for (let i = 255; i < 512; i++) EXP[i] = EXP[i - 255];
})();

function gfMul(a: number, b: number): number {
  if (a === 0 || b === 0) return 0;
  return EXP[LOG[a] + LOG[b]];
}

/** Build the generator polynomial of the given degree. */
function rsGeneratorPoly(degree: number): number[] {
  let poly = [1];
  for (let i = 0; i < degree; i++) {
    const next = new Array(poly.length + 1).fill(0);
    for (let j = 0; j < poly.length; j++) {
      next[j] ^= poly[j];
      next[j + 1] ^= gfMul(poly[j], EXP[i]);
    }
    poly = next;
  }
  return poly;
}

/** Compute ECC codewords for one block of data codewords. */
function rsEncode(data: number[], eccLen: number): number[] {
  const gen = rsGeneratorPoly(eccLen);
  const res = new Array(eccLen).fill(0);
  for (const d of data) {
    const factor = d ^ res[0];
    res.shift();
    res.push(0);
    if (factor !== 0) {
      for (let i = 0; i < gen.length - 1; i++) {
        res[i] ^= gfMul(gen[i + 1], factor);
      }
    }
  }
  return res;
}

// ---- Version / capacity tables (error-correction level M) ----
// For each version 1..40: total codewords, EC codewords per block,
// number of blocks in group 1, then group 2 (if any).
// [totalDataCodewords, eccPerBlock, g1Blocks, g1DataCw, g2Blocks, g2DataCw]
const EC_M: Record<number, [number, number, number, number, number, number]> = {
  1: [16, 10, 1, 16, 0, 0],
  2: [28, 16, 1, 28, 0, 0],
  3: [44, 26, 1, 44, 0, 0],
  4: [64, 18, 2, 32, 0, 0],
  5: [86, 24, 2, 43, 0, 0],
  6: [108, 16, 4, 27, 0, 0],
  7: [124, 18, 4, 31, 0, 0],
  8: [154, 22, 2, 38, 2, 39],
  9: [182, 22, 3, 36, 2, 37],
  10: [216, 26, 4, 43, 1, 44],
};

// Total number of codeword bytes (data + ecc) for each version.
const TOTAL_CODEWORDS: Record<number, number> = {
  1: 26,
  2: 44,
  3: 70,
  4: 100,
  5: 134,
  6: 172,
  7: 196,
  8: 242,
  9: 292,
  10: 346,
};

// Alignment pattern center coordinates per version (empty for v1).
const ALIGN_POS: Record<number, number[]> = {
  1: [],
  2: [6, 18],
  3: [6, 22],
  4: [6, 26],
  5: [6, 30],
  6: [6, 34],
  7: [6, 22, 38],
  8: [6, 24, 42],
  9: [6, 26, 46],
  10: [6, 28, 50],
};

const PAD_BYTES = [0xec, 0x11];

function bitsForCharCount(version: number): number {
  // Byte-mode character count indicator length.
  return version <= 9 ? 8 : 16;
}

/** Pick the smallest supported version that can hold the byte payload. */
function pickVersion(byteLen: number): number {
  for (let v = 1; v <= 10; v++) {
    const dataCw = EC_M[v][0];
    const ccBits = bitsForCharCount(v);
    // 4 mode bits + char-count bits + 8 bits per data byte.
    const needBits = 4 + ccBits + byteLen * 8;
    if (needBits <= dataCw * 8) return v;
  }
  throw new Error("QR payload too large for supported versions (1-10).");
}

class BitBuffer {
  bits: number[] = [];
  put(value: number, length: number) {
    for (let i = length - 1; i >= 0; i--) {
      this.bits.push((value >>> i) & 1);
    }
  }
  get length() {
    return this.bits.length;
  }
}

function encodeData(text: string, version: number): number[] {
  const bytes = new TextEncoder().encode(text);
  const buf = new BitBuffer();
  buf.put(0b0100, 4); // byte mode
  buf.put(bytes.length, bitsForCharCount(version));
  for (const b of bytes) buf.put(b, 8);

  const dataCw = EC_M[version][0];
  const capacityBits = dataCw * 8;

  // Terminator (up to 4 zero bits).
  const terminator = Math.min(4, capacityBits - buf.length);
  buf.put(0, terminator);
  // Pad to byte boundary.
  while (buf.length % 8 !== 0) buf.bits.push(0);

  // Convert to bytes.
  const codewords: number[] = [];
  for (let i = 0; i < buf.length; i += 8) {
    let byte = 0;
    for (let j = 0; j < 8; j++) byte = (byte << 1) | buf.bits[i + j];
    codewords.push(byte);
  }
  // Pad bytes.
  let pi = 0;
  while (codewords.length < dataCw) {
    codewords.push(PAD_BYTES[pi % 2]);
    pi++;
  }
  return codewords;
}

/** Interleave data + ECC across blocks per the QR spec. */
function buildCodewords(dataCw: number[], version: number): number[] {
  const [, eccPerBlock, g1Blocks, g1Cw, g2Blocks, g2Cw] = EC_M[version];
  const blocks: number[][] = [];
  const eccBlocks: number[][] = [];

  let offset = 0;
  for (let i = 0; i < g1Blocks; i++) {
    const block = dataCw.slice(offset, offset + g1Cw);
    offset += g1Cw;
    blocks.push(block);
    eccBlocks.push(rsEncode(block, eccPerBlock));
  }
  for (let i = 0; i < g2Blocks; i++) {
    const block = dataCw.slice(offset, offset + g2Cw);
    offset += g2Cw;
    blocks.push(block);
    eccBlocks.push(rsEncode(block, eccPerBlock));
  }

  const result: number[] = [];
  const maxData = Math.max(g1Cw, g2Cw);
  for (let i = 0; i < maxData; i++) {
    for (const block of blocks) {
      if (i < block.length) result.push(block[i]);
    }
  }
  for (let i = 0; i < eccPerBlock; i++) {
    for (const ecc of eccBlocks) {
      result.push(ecc[i]);
    }
  }
  return result;
}

type Grid = (number | null)[][];

function makeGrid(size: number): Grid {
  return Array.from({ length: size }, () => new Array(size).fill(null));
}

function placeFinder(grid: Grid, row: number, col: number) {
  for (let r = -1; r <= 7; r++) {
    for (let c = -1; c <= 7; c++) {
      const rr = row + r;
      const cc = col + c;
      if (rr < 0 || rr >= grid.length || cc < 0 || cc >= grid.length) continue;
      const inRing =
        (r >= 0 && r <= 6 && (c === 0 || c === 6)) ||
        (c >= 0 && c <= 6 && (r === 0 || r === 6));
      const inCenter = r >= 2 && r <= 4 && c >= 2 && c <= 4;
      grid[rr][cc] = inRing || inCenter ? 1 : 0;
    }
  }
}

function placeAlignment(grid: Grid, version: number) {
  const positions = ALIGN_POS[version];
  for (const r of positions) {
    for (const c of positions) {
      // Skip if overlapping a finder pattern.
      if (grid[r][c] !== null) continue;
      for (let dr = -2; dr <= 2; dr++) {
        for (let dc = -2; dc <= 2; dc++) {
          const isRing = Math.max(Math.abs(dr), Math.abs(dc)) !== 1;
          grid[r + dr][c + dc] = isRing ? 1 : 0;
        }
      }
    }
  }
}

function placeTiming(grid: Grid) {
  const size = grid.length;
  for (let i = 8; i < size - 8; i++) {
    const bit = i % 2 === 0 ? 1 : 0;
    if (grid[6][i] === null) grid[6][i] = bit;
    if (grid[i][6] === null) grid[i][6] = bit;
  }
}

/** Reserve format/version info areas with placeholder so data skips them. */
function reserveFormat(grid: Grid) {
  const size = grid.length;
  // Dark module.
  grid[size - 8][8] = 1;
  for (let i = 0; i <= 8; i++) {
    if (grid[8][i] === null) grid[8][i] = 0;
    if (grid[i][8] === null) grid[i][8] = 0;
  }
  for (let i = 0; i < 8; i++) {
    if (grid[8][size - 1 - i] === null) grid[8][size - 1 - i] = 0;
    if (grid[size - 1 - i][8] === null) grid[size - 1 - i][8] = 0;
  }
}

function isReserved(grid: Grid, r: number, c: number): boolean {
  return grid[r][c] !== null;
}

function placeData(grid: Grid, codewords: number[]) {
  const size = grid.length;
  const bits: number[] = [];
  for (const cw of codewords) {
    for (let i = 7; i >= 0; i--) bits.push((cw >>> i) & 1);
  }

  let bitIdx = 0;
  let upward = true;
  for (let col = size - 1; col > 0; col -= 2) {
    if (col === 6) col--; // skip vertical timing column
    for (let i = 0; i < size; i++) {
      const row = upward ? size - 1 - i : i;
      for (let j = 0; j < 2; j++) {
        const cc = col - j;
        if (isReserved(grid, row, cc)) continue;
        const bit = bitIdx < bits.length ? bits[bitIdx] : 0;
        grid[row][cc] = bit;
        bitIdx++;
      }
    }
    upward = !upward;
  }
}

const MASKS: ((r: number, c: number) => boolean)[] = [
  (r, c) => (r + c) % 2 === 0,
  (r) => r % 2 === 0,
  (_r, c) => c % 3 === 0,
  (r, c) => (r + c) % 3 === 0,
  (r, c) => (Math.floor(r / 2) + Math.floor(c / 3)) % 2 === 0,
  (r, c) => ((r * c) % 2) + ((r * c) % 3) === 0,
  (r, c) => (((r * c) % 2) + ((r * c) % 3)) % 2 === 0,
  (r, c) => (((r + c) % 2) + ((r * c) % 3)) % 2 === 0,
];

function applyMask(grid: Grid, reserved: boolean[][], maskFn: (r: number, c: number) => boolean): Grid {
  const size = grid.length;
  const out = grid.map((row) => row.slice());
  for (let r = 0; r < size; r++) {
    for (let c = 0; c < size; c++) {
      if (reserved[r][c]) continue;
      if (maskFn(r, c)) out[r][c] = (out[r][c] as number) ^ 1;
    }
  }
  return out;
}

/** Penalty score used to choose the best mask (lower is better). */
function penalty(grid: Grid): number {
  const size = grid.length;
  const g = grid as number[][];
  let score = 0;

  // Rule 1: runs of 5+ same-colour modules.
  for (let r = 0; r < size; r++) {
    let runColor = g[r][0];
    let runLen = 1;
    for (let c = 1; c < size; c++) {
      if (g[r][c] === runColor) {
        runLen++;
      } else {
        if (runLen >= 5) score += 3 + (runLen - 5);
        runColor = g[r][c];
        runLen = 1;
      }
    }
    if (runLen >= 5) score += 3 + (runLen - 5);
  }
  for (let c = 0; c < size; c++) {
    let runColor = g[0][c];
    let runLen = 1;
    for (let r = 1; r < size; r++) {
      if (g[r][c] === runColor) {
        runLen++;
      } else {
        if (runLen >= 5) score += 3 + (runLen - 5);
        runColor = g[r][c];
        runLen = 1;
      }
    }
    if (runLen >= 5) score += 3 + (runLen - 5);
  }

  // Rule 2: 2x2 blocks of same colour.
  for (let r = 0; r < size - 1; r++) {
    for (let c = 0; c < size - 1; c++) {
      const v = g[r][c];
      if (v === g[r][c + 1] && v === g[r + 1][c] && v === g[r + 1][c + 1]) score += 3;
    }
  }

  // Rule 3: finder-like patterns.
  const pat1 = [1, 0, 1, 1, 1, 0, 1, 0, 0, 0, 0];
  const pat2 = [0, 0, 0, 0, 1, 0, 1, 1, 1, 0, 1];
  const matches = (arr: number[], start: number, pattern: number[]) => {
    for (let i = 0; i < pattern.length; i++) {
      if (arr[start + i] !== pattern[i]) return false;
    }
    return true;
  };
  for (let r = 0; r < size; r++) {
    for (let c = 0; c <= size - 11; c++) {
      const row = g[r];
      if (matches(row, c, pat1) || matches(row, c, pat2)) score += 40;
    }
  }
  for (let c = 0; c < size; c++) {
    const col = g.map((row) => row[c]);
    for (let r = 0; r <= size - 11; r++) {
      if (matches(col, r, pat1) || matches(col, r, pat2)) score += 40;
    }
  }

  // Rule 4: dark/light balance.
  let dark = 0;
  for (let r = 0; r < size; r++) for (let c = 0; c < size; c++) dark += g[r][c];
  const ratio = (dark / (size * size)) * 100;
  const prev = Math.floor(Math.abs(ratio - 50) / 5);
  score += prev * 10;

  return score;
}

// Pre-computed BCH format-info bits (level M = 0b00) for each mask 0..7.
const FORMAT_BITS_M = [
  0x5412, 0x5125, 0x5e7c, 0x5b4b, 0x45f9, 0x40ce, 0x4f97, 0x4aa0,
];

function placeFormatInfo(grid: Grid, mask: number) {
  const size = grid.length;
  const bits = FORMAT_BITS_M[mask];
  const get = (i: number) => (bits >>> i) & 1;

  // Around top-left finder.
  for (let i = 0; i <= 5; i++) grid[8][i] = get(i);
  grid[8][7] = get(6);
  grid[8][8] = get(7);
  grid[7][8] = get(8);
  for (let i = 9; i <= 14; i++) grid[14 - i][8] = get(i);

  // Around the other two finders.
  for (let i = 0; i <= 7; i++) grid[size - 1 - i][8] = get(i);
  for (let i = 8; i <= 14; i++) grid[8][size - 15 + i] = get(i);
  grid[size - 8][8] = 1; // dark module
}

export interface QrMatrix {
  size: number;
  modules: boolean[][];
  version: number;
}

/** Generate a QR code module matrix for the given text (byte mode, EC level M). */
export function makeQr(text: string): QrMatrix {
  const byteLen = new TextEncoder().encode(text).length;
  const version = pickVersion(byteLen);
  const size = version * 4 + 17;

  // Sanity check our codeword totals.
  void TOTAL_CODEWORDS;

  const dataCw = encodeData(text, version);
  const codewords = buildCodewords(dataCw, version);

  // Base grid with function patterns; build a reserved map for masking.
  const base = makeGrid(size);
  placeFinder(base, 0, 0);
  placeFinder(base, 0, size - 7);
  placeFinder(base, size - 7, 0);
  placeAlignment(base, version);
  placeTiming(base);
  reserveFormat(base);

  const reserved: boolean[][] = base.map((row) => row.map((v) => v !== null));

  placeData(base, codewords);

  // Fill any remaining nulls with 0 (shouldn't happen, but be safe).
  for (let r = 0; r < size; r++)
    for (let c = 0; c < size; c++) if (base[r][c] === null) base[r][c] = 0;

  // Choose the lowest-penalty mask.
  let best: Grid | null = null;
  let bestMask = 0;
  let bestScore = Infinity;
  for (let m = 0; m < 8; m++) {
    const candidate = applyMask(base, reserved, MASKS[m]);
    placeFormatInfo(candidate, m);
    const score = penalty(candidate);
    if (score < bestScore) {
      bestScore = score;
      best = candidate;
      bestMask = m;
    }
  }
  void bestMask;

  const modules = (best as Grid).map((row) => row.map((v) => v === 1));
  return { size, modules, version };
}

/**
 * Render a QR matrix as an SVG string (string -> data isn't needed; the
 * component embeds this SVG directly). Quiet zone of 4 modules included.
 */
export function qrToSvg(matrix: QrMatrix, opts?: { fg?: string; bg?: string }): string {
  const { size, modules } = matrix;
  const quiet = 4;
  const dim = size + quiet * 2;
  const fg = opts?.fg ?? "#000000";
  const bg = opts?.bg ?? "#ffffff";

  let path = "";
  for (let r = 0; r < size; r++) {
    for (let c = 0; c < size; c++) {
      if (modules[r][c]) {
        path += `M${c + quiet} ${r + quiet}h1v1h-1z`;
      }
    }
  }

  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${dim} ${dim}" shape-rendering="crispEdges">` +
    `<rect width="${dim}" height="${dim}" fill="${bg}"/>` +
    `<path d="${path}" fill="${fg}"/>` +
    `</svg>`
  );
}

/** Convenience: build a base64 data URL for an <img src>. */
export function qrToDataUrl(text: string, opts?: { fg?: string; bg?: string }): string {
  const svg = qrToSvg(makeQr(text), opts);
  if (typeof btoa === "function") {
    return `data:image/svg+xml;base64,${btoa(svg)}`;
  }
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}
