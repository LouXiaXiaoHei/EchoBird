// A fixed glyph grid lit by drifting, curled ribbons. One instanced draw keeps
// the ambient effect off the React render path and avoids a full-size simulation.
const VERTEX = `#version 300 es
precision highp float;
uniform vec2 u_size;
uniform vec2 u_grid;
uniform vec2 u_origin;
uniform float u_time;
uniform float u_seed;
uniform float u_glyphSeed;
uniform float u_age;
out vec2 v_uv;
out float v_light;

float hash(vec2 p) {
  return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}
float noise(vec2 p) {
  vec2 i = floor(p), f = fract(p);
  f = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash(i), hash(i + vec2(1, 0)), f.x),
             mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x), f.y);
}
float curl(vec2 p) {
  return noise(p) * 0.62 + noise(p * 2.1 + 9.0) * 0.27
       + noise(p * 4.3 + 17.0) * 0.11;
}
void main() {
  float index = float(gl_InstanceID);
  vec2 cell = vec2(mod(index, u_grid.x), floor(index / u_grid.x));
  vec2 corner = vec2(float(gl_VertexID % 2), float(gl_VertexID / 2));
  vec2 position = (cell + corner) / u_grid;
  gl_Position = vec4(position * vec2(2, -2) + vec2(-1, 1), 0, 1);

  vec2 p = (cell + 0.5) * vec2(7, 14) / 230.0;
  float t = u_time * 0.14;
  vec2 drift = vec2(t * 0.8, -t * 0.35);
  float eddy = curl(p * 0.72 + drift + u_seed);
  float fold = sin(p.x * 0.8 + p.y * 0.52 + eddy * 5.0 - t * 1.7);
  float ribbon = pow(max(0.0, 1.0 - abs(fold) * 3.2), 2.0);
  float detail = curl(p * 1.8 - drift * 0.7 + 24.0);
  float body = smoothstep(0.33, 0.76, eddy) * (0.3 + detail * 0.7);
  vec2 delta = ((cell + 0.5) / u_grid - u_origin) * u_size / 340.0;
  float distance = length(delta);
  float wake = exp(-pow((distance - u_age * 0.42) * 2.4, 2.0))
             * exp(-u_age * 0.48);
  v_light = clamp(0.004 + body * 0.07 + ribbon * detail * 0.42 + wake * 0.20, 0.0, 0.58);
  v_light *= smoothstep(0.0, 1.5, u_time);

  // Only scattered glyphs change at once; the wind carries light rather than
  // scrolling whole columns of text down the screen.
  float tick = floor(u_time * 0.65 + hash(cell) * 60.0);
  float glyph = floor(hash(cell + vec2(tick, u_glyphSeed)) * 62.0);
  vec2 tile = vec2(mod(glyph, 16.0), floor(glyph / 16.0));
  v_uv = (tile + corner) / vec2(16, 4);
}`;

const FRAGMENT = `#version 300 es
precision mediump float;
uniform sampler2D u_atlas;
uniform vec4 u_ink;
in vec2 v_uv;
in float v_light;
out vec4 color;
void main() {
  float alpha = texture(u_atlas, v_uv).a * v_light * u_ink.a;
  color = vec4(u_ink.rgb * alpha, alpha);
}`;

export interface CharacterRenderer {
  select: (id: string, origin: { x: number; y: number }) => void;
  dispose: () => void;
}

type RGB = [number, number, number];

export function characterInk(
  surface: RGB,
  accent: RGB,
  dark: boolean
): [number, number, number, number] {
  // Background palettes carry the theme hue; neutral palettes use the UI accent.
  const source = Math.max(...surface) - Math.min(...surface) < 18 ? accent : surface;
  const minimum = Math.min(...source);
  const range = Math.max(...source) - minimum;
  const base = dark ? 0.22 : 0.1;
  const span = dark ? 0.66 : 0.43;
  const channels = source.map(
    (channel) => base + (range ? (channel - minimum) / range : 0.5) * span
  );
  return [channels[0], channels[1], channels[2], dark ? 1 : 0.48];
}

export function createCharacterRenderer(canvas: HTMLCanvasElement): CharacterRenderer | null {
  let gl: WebGL2RenderingContext | null;
  try {
    gl = canvas.getContext('webgl2', {
      alpha: true,
      antialias: false,
      depth: false,
      powerPreference: 'low-power',
    });
  } catch {
    return null;
  }
  if (!gl) return null;
  const context = gl;
  const shaders: WebGLShader[] = [];
  let program: WebGLProgram | null = null;
  let atlas: WebGLTexture | null = null;
  let frame = 0;
  let disposed = false;
  let observer: ResizeObserver | null = null;
  let theme: MutationObserver | null = null;
  const onContextLost = (event: Event) => {
    event.preventDefault();
    canvas.style.visibility = 'hidden';
    release();
  };
  const release = () => {
    if (disposed) return;
    disposed = true;
    cancelAnimationFrame(frame);
    observer?.disconnect();
    theme?.disconnect();
    canvas.removeEventListener('webglcontextlost', onContextLost);
    shaders.forEach((shader) => context.deleteShader(shader));
    context.deleteTexture(atlas);
    context.deleteProgram(program);
    if (!context.isContextLost()) context.getExtension('WEBGL_lose_context')?.loseContext();
  };

  try {
    for (const [type, source] of [
      [context.VERTEX_SHADER, VERTEX],
      [context.FRAGMENT_SHADER, FRAGMENT],
    ] as const) {
      const shader = context.createShader(type);
      if (!shader) throw new Error('Unable to allocate character shader');
      shaders.push(shader);
      context.shaderSource(shader, source);
      context.compileShader(shader);
      if (!context.getShaderParameter(shader, context.COMPILE_STATUS)) {
        throw new Error(context.getShaderInfoLog(shader) || 'Character shader compilation failed');
      }
    }
    program = context.createProgram();
    if (!program) throw new Error('Unable to allocate character program');
    shaders.forEach((shader) => context.attachShader(program!, shader));
    context.linkProgram(program);
    if (!context.getProgramParameter(program, context.LINK_STATUS)) {
      throw new Error(context.getProgramInfoLog(program) || 'Character shader link failed');
    }

    const glyphs = document.createElement('canvas');
    glyphs.width = 16 * 28;
    glyphs.height = 4 * 56;
    const ink = glyphs.getContext('2d');
    if (!ink) throw new Error('Character atlas unavailable');
    ink.font = '38px "JetBrains Mono", monospace';
    ink.textAlign = 'center';
    ink.textBaseline = 'middle';
    ink.fillStyle = '#fff';
    const alphabet = '0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ';
    for (let i = 0; i < alphabet.length; i++) {
      ink.fillText(alphabet[i], (i % 16) * 28 + 14, Math.floor(i / 16) * 56 + 28);
    }
    atlas = context.createTexture();
    if (!atlas) throw new Error('Unable to allocate character atlas');
    context.bindTexture(context.TEXTURE_2D, atlas);
    context.texParameteri(context.TEXTURE_2D, context.TEXTURE_MIN_FILTER, context.LINEAR);
    context.texParameteri(context.TEXTURE_2D, context.TEXTURE_MAG_FILTER, context.LINEAR);
    context.texParameteri(context.TEXTURE_2D, context.TEXTURE_WRAP_S, context.CLAMP_TO_EDGE);
    context.texParameteri(context.TEXTURE_2D, context.TEXTURE_WRAP_T, context.CLAMP_TO_EDGE);
    context.texImage2D(
      context.TEXTURE_2D,
      0,
      context.RGBA,
      context.RGBA,
      context.UNSIGNED_BYTE,
      glyphs
    );
    context.useProgram(program);
    const uniforms = Object.fromEntries(
      ['size', 'grid', 'origin', 'time', 'seed', 'glyphSeed', 'age', 'ink'].map((name) => [
        name,
        context.getUniformLocation(program!, `u_${name}`),
      ])
    );
    let width = 1;
    let height = 1;
    let columns = 1;
    let rows = 1;
    let seed = 1;
    let targetSeed = 1;
    let origin = { x: 0.5, y: 0.5 };
    const start = performance.now();
    let selectedAt = start;
    let lastFrame = -Infinity;
    let color: [number, number, number, number];
    const refreshTheme = () => {
      const style = getComputedStyle(document.documentElement);
      const read = (name: string) =>
        style.getPropertyValue(name).trim().split(/\s+/).map(Number) as RGB;
      color = characterInk(
        read('--bg-elevated-rgb'),
        read('--accent-rgb'),
        document.documentElement.dataset.theme !== 'light'
      );
    };
    refreshTheme();
    theme = new MutationObserver(refreshTheme);
    theme.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-theme', 'style'],
    });
    const resize = () => {
      const rect = canvas.getBoundingClientRect();
      width = Math.max(1, rect.width);
      height = Math.max(1, rect.height);
      const dpr = Math.min(
        window.devicePixelRatio || 1,
        1.5,
        Math.sqrt(4_000_000 / (width * height))
      );
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
      // Keep the glyph count bounded on large displays, independently of DPR.
      const cell = Math.max(8, Math.sqrt((width * height) / 32000));
      columns = Math.max(1, Math.floor(width / cell));
      rows = Math.max(1, Math.floor(height / (cell * 2)));
      context.viewport(0, 0, canvas.width, canvas.height);
    };
    observer = new ResizeObserver(resize);
    observer.observe(canvas);
    resize();
    const draw = (now: number) => {
      if (disposed) return;
      frame = requestAnimationFrame(draw);
      if (now - lastFrame < 40) return;
      const delta = Math.min(100, now - (Number.isFinite(lastFrame) ? lastFrame : now));
      lastFrame = now;
      seed += (targetSeed - seed) * (1 - Math.exp(-delta / 1800));
      context.uniform2f(uniforms.size, width, height);
      context.uniform2f(uniforms.grid, columns, rows);
      context.uniform2f(uniforms.origin, origin.x, origin.y);
      context.uniform1f(uniforms.time, (now - start) / 1000);
      context.uniform1f(uniforms.age, (now - selectedAt) / 1000);
      context.uniform1f(uniforms.seed, seed);
      context.uniform1f(uniforms.glyphSeed, targetSeed);
      context.uniform4f(uniforms.ink, ...color);
      context.drawArraysInstanced(context.TRIANGLE_STRIP, 0, 4, columns * rows);
    };
    canvas.addEventListener('webglcontextlost', onContextLost);
    frame = requestAnimationFrame(draw);
    return {
      select(id, position) {
        let hash = 0;
        for (const character of id) hash = (Math.imul(hash, 31) + character.charCodeAt(0)) | 0;
        targetSeed = ((hash >>> 0) % 9700) / 2425;
        origin = position;
        selectedAt = performance.now();
      },
      dispose: release,
    };
  } catch {
    // Decoration must never block selection or launching on unsupported GPUs.
    release();
    return null;
  }
}
