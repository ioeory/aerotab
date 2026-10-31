/**
 * WebGL2 grid renderer: solid background quads + glyph-atlas textured quads.
 * Falls back is the caller's responsibility when context creation fails.
 */

import { cellStyle, type CursorState } from './cellStyle';
import {
  CURSOR_BEAM,
  CURSOR_BLOCK,
  CURSOR_HOLLOW,
  CURSOR_UNDERLINE,
} from './frame';
import { GlyphAtlas } from './glyphAtlas';
import type { CellMetrics, FontSpec } from './fontMetrics';
import type { Palette } from './palette';
import type { GridRenderer, RenderScene } from './renderer';

const BG_VS = `#version 300 es
precision highp float;
layout(location=0) in vec2 a_pos;
layout(location=1) in vec4 a_color;
uniform vec2 u_res;
out vec4 v_color;
void main() {
  vec2 p = a_pos / u_res * 2.0 - 1.0;
  gl_Position = vec4(p.x, -p.y, 0.0, 1.0);
  v_color = a_color;
}`;

const BG_FS = `#version 300 es
precision mediump float;
in vec4 v_color;
out vec4 o;
void main() { o = v_color; }`;

const GLYPH_VS = `#version 300 es
precision highp float;
layout(location=0) in vec2 a_pos;
layout(location=1) in vec2 a_uv;
layout(location=2) in vec4 a_color;
uniform vec2 u_res;
out vec2 v_uv;
out vec4 v_color;
void main() {
  vec2 p = a_pos / u_res * 2.0 - 1.0;
  gl_Position = vec4(p.x, -p.y, 0.0, 1.0);
  v_uv = a_uv;
  v_color = a_color;
}`;

const GLYPH_FS = `#version 300 es
precision mediump float;
in vec2 v_uv;
in vec4 v_color;
uniform sampler2D u_atlas;
out vec4 o;
void main() {
  float a = texture(u_atlas, v_uv).a;
  o = vec4(v_color.rgb, v_color.a * a);
}`;

function compile(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader {
  const s = gl.createShader(type)!;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(s) ?? 'compile failed';
    gl.deleteShader(s);
    throw new Error(`termgrid webgl: ${log}`);
  }
  return s;
}

function link(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const prog = gl.createProgram()!;
  const v = compile(gl, gl.VERTEX_SHADER, vs);
  const f = compile(gl, gl.FRAGMENT_SHADER, fs);
  gl.attachShader(prog, v);
  gl.attachShader(prog, f);
  gl.linkProgram(prog);
  gl.deleteShader(v);
  gl.deleteShader(f);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(prog) ?? 'link failed';
    gl.deleteProgram(prog);
    throw new Error(`termgrid webgl: ${log}`);
  }
  return prog;
}

function rgba(rgb: number, a = 1): [number, number, number, number] {
  return [((rgb >> 16) & 0xff) / 255, ((rgb >> 8) & 0xff) / 255, (rgb & 0xff) / 255, a];
}

export class WebGLRenderer implements GridRenderer {
  readonly kind = 'webgl' as const;
  private canvas: HTMLCanvasElement;
  private gl: WebGL2RenderingContext;
  private bgProg: WebGLProgram;
  private glyphProg: WebGLProgram;
  private bgVao: WebGLVertexArrayObject;
  private glyphVao: WebGLVertexArrayObject;
  private bgBuf: WebGLBuffer;
  private glyphBuf: WebGLBuffer;
  private atlasTex: WebGLTexture;
  private atlas: GlyphAtlas;
  private atlasGen = -1;
  private cols = 0;
  private rows = 0;
  private metrics: CellMetrics;
  private spec: FontSpec;
  private dpr = 1;
  private cssW = 0;
  private cssH = 0;

  constructor(canvas: HTMLCanvasElement, spec: FontSpec, metrics: CellMetrics) {
    this.canvas = canvas;
    this.spec = spec;
    this.metrics = metrics;
    const gl = canvas.getContext('webgl2', {
      alpha: true,
      antialias: false,
      premultipliedAlpha: false,
      preserveDrawingBuffer: false,
    });
    if (!gl) throw new Error('termgrid: webgl2 unavailable');
    this.gl = gl;
    this.bgProg = link(gl, BG_VS, BG_FS);
    this.glyphProg = link(gl, GLYPH_VS, GLYPH_FS);
    this.bgVao = gl.createVertexArray()!;
    this.glyphVao = gl.createVertexArray()!;
    this.bgBuf = gl.createBuffer()!;
    this.glyphBuf = gl.createBuffer()!;
    this.atlasTex = gl.createTexture()!;
    this.atlas = new GlyphAtlas(spec, metrics);

    gl.bindVertexArray(this.bgVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.bgBuf);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 24, 0);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 4, gl.FLOAT, false, 24, 8);

    gl.bindVertexArray(this.glyphVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.glyphBuf);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 32, 0);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 32, 8);
    gl.enableVertexAttribArray(2);
    gl.vertexAttribPointer(2, 4, gl.FLOAT, false, 32, 16);

    gl.bindTexture(gl.TEXTURE_2D, this.atlasTex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  }

  resize(cols: number, rows: number, metrics: CellMetrics, dpr: number) {
    this.cols = cols;
    this.rows = rows;
    this.metrics = metrics;
    this.dpr = Math.max(1, dpr);
    this.cssW = cols * metrics.width;
    this.cssH = rows * metrics.height;
    this.canvas.width = Math.max(1, Math.round(cols * metrics.deviceWidth));
    this.canvas.height = Math.max(1, Math.round(rows * metrics.deviceHeight));
    this.canvas.style.width = `${this.cssW}px`;
    this.canvas.style.height = `${this.cssH}px`;
    this.gl.viewport(0, 0, this.canvas.width, this.canvas.height);
  }

  setFont(spec: FontSpec, metrics: CellMetrics) {
    this.spec = spec;
    this.metrics = metrics;
    this.atlas.setFont(spec, metrics);
  }

  render(scene: RenderScene) {
    const { grid, palette, cursor } = scene;
    const gl = this.gl;
    const cw = this.metrics.width;
    const ch = this.metrics.height;
    const [br, bg, bb] = rgba(palette.background);
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.clearColor(br, bg, bb, palette.backgroundAlpha);
    gl.clear(gl.COLOR_BUFFER_BIT);

    const bgVerts: number[] = [];
    const glyphVerts: number[] = [];

    for (let y = 0; y < grid.rows; y++) {
      const base = y * grid.cols;
      for (let x = 0; x < grid.cols; x++) {
        const idx = base + x;
        const style = cellStyle(grid, idx, x, y, palette, cursor);
        const x0 = x * cw;
        const y0 = y * ch;
        const w = cw * (style.wide ? 2 : 1);
        if (style.paintBg) {
          pushQuadColor(bgVerts, x0, y0, w, ch, rgba(style.bg));
        }
        if (!style.drawGlyph) continue;
        const cp = grid.chars[idx] ?? 0x20;
        const g = this.atlas.get(cp, style.bold, style.italic, style.wide);
        if (!g) continue;
        const atlasSize = this.atlas.size;
        const u0 = g.x / atlasSize;
        const v0 = g.y / atlasSize;
        const u1 = (g.x + g.w) / atlasSize;
        const v1 = (g.y + g.h) / atlasSize;
        const gx = x0 + g.dx / this.dpr;
        const gy = y0 + g.dx / this.dpr; // pad is symmetric; top also uses PAD
        // Prefer exact cell placement; atlas glyph includes PAD so offset by dx.
        const gw = g.w / this.dpr;
        const gh = g.h / this.dpr;
        pushQuadGlyph(glyphVerts, gx, gy, gw, gh, u0, v0, u1, v1, rgba(style.fg));
        if (style.underline || style.doubleUnderline) {
          pushQuadColor(bgVerts, x0, y0 + ch - 1.5, w, 1, rgba(style.fg));
          if (style.doubleUnderline) {
            pushQuadColor(bgVerts, x0, y0 + ch - 3.5, w, 1, rgba(style.fg));
          }
        }
        if (style.strike) {
          pushQuadColor(bgVerts, x0, y0 + ch * 0.5, w, 1, rgba(style.fg));
        }
      }
    }

    this.drawNonBlockCursor(bgVerts, cursor, cw, ch, palette);

    // Upload atlas if dirty. Sample alpha — Canvas 2D AA lives in the A channel.
    if (this.atlas.dirty || this.atlas.generation !== this.atlasGen) {
      gl.bindTexture(gl.TEXTURE_2D, this.atlasTex);
      gl.texImage2D(
        gl.TEXTURE_2D,
        0,
        gl.RGBA,
        gl.RGBA,
        gl.UNSIGNED_BYTE,
        this.atlas.canvas as TexImageSource,
      );
      this.atlas.markUploaded();
      this.atlasGen = this.atlas.generation;
    }

    const resCss: [number, number] = [this.cssW || 1, this.cssH || 1];

    if (bgVerts.length) {
      gl.useProgram(this.bgProg);
      gl.uniform2f(gl.getUniformLocation(this.bgProg, 'u_res'), resCss[0], resCss[1]);
      gl.bindVertexArray(this.bgVao);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.bgBuf);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(bgVerts), gl.DYNAMIC_DRAW);
      gl.drawArrays(gl.TRIANGLES, 0, bgVerts.length / 6);
    }

    if (glyphVerts.length) {
      gl.useProgram(this.glyphProg);
      gl.uniform2f(gl.getUniformLocation(this.glyphProg, 'u_res'), resCss[0], resCss[1]);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, this.atlasTex);
      gl.uniform1i(gl.getUniformLocation(this.glyphProg, 'u_atlas'), 0);
      gl.bindVertexArray(this.glyphVao);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.glyphBuf);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(glyphVerts), gl.DYNAMIC_DRAW);
      gl.drawArrays(gl.TRIANGLES, 0, glyphVerts.length / 8);
    }

    grid.clearDirty();
  }

  private drawNonBlockCursor(
    verts: number[],
    cursor: CursorState | null,
    cw: number,
    ch: number,
    palette: Palette,
  ) {
    if (!cursor || !cursor.visible || cursor.shape === CURSOR_BLOCK) return;
    if (cursor.col < 0 || cursor.line < 0) return;
    const x = cursor.col * cw;
    const y = cursor.line * ch;
    const c = rgba(palette.cursor);
    switch (cursor.shape) {
      case CURSOR_UNDERLINE:
        pushQuadColor(verts, x, y + ch - 2, cw, 2, c);
        break;
      case CURSOR_BEAM:
        pushQuadColor(verts, x, y, 2, ch, c);
        break;
      case CURSOR_HOLLOW: {
        pushQuadColor(verts, x, y, cw, 1, c);
        pushQuadColor(verts, x, y + ch - 1, cw, 1, c);
        pushQuadColor(verts, x, y, 1, ch, c);
        pushQuadColor(verts, x + cw - 1, y, 1, ch, c);
        break;
      }
      default:
        break;
    }
  }

  dispose() {
    const gl = this.gl;
    gl.deleteProgram(this.bgProg);
    gl.deleteProgram(this.glyphProg);
    gl.deleteBuffer(this.bgBuf);
    gl.deleteBuffer(this.glyphBuf);
    gl.deleteVertexArray(this.bgVao);
    gl.deleteVertexArray(this.glyphVao);
    gl.deleteTexture(this.atlasTex);
    this.atlas.clear();
  }
}

function pushQuadColor(
  out: number[],
  x: number,
  y: number,
  w: number,
  h: number,
  c: [number, number, number, number],
) {
  const x1 = x + w;
  const y1 = y + h;
  // two triangles
  out.push(x, y, ...c, x1, y, ...c, x, y1, ...c);
  out.push(x, y1, ...c, x1, y, ...c, x1, y1, ...c);
}

function pushQuadGlyph(
  out: number[],
  x: number,
  y: number,
  w: number,
  h: number,
  u0: number,
  v0: number,
  u1: number,
  v1: number,
  c: [number, number, number, number],
) {
  const x1 = x + w;
  const y1 = y + h;
  out.push(x, y, u0, v0, ...c, x1, y, u1, v0, ...c, x, y1, u0, v1, ...c);
  out.push(x, y1, u0, v1, ...c, x1, y, u1, v0, ...c, x1, y1, u1, v1, ...c);
}
