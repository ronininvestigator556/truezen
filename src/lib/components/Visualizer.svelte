<script lang="ts">
  /**
   * Waveform, spectrum and beat envelope on one canvas.
   *
   * Beyond looking good this is the fastest way to confirm the engine is doing
   * what the numbers claim: a binaural layer shows two spectral peaks, an
   * isochronic one shows the envelope pulsing at the beat rate.
   */
  interface Props {
    wave: number[];
    env: number[];
    sampleRate: number;
    playing: boolean;
  }
  let { wave, env, sampleRate, playing }: Props = $props();

  let canvas: HTMLCanvasElement | null = $state(null);

  /** Radix-2 FFT, iterative. Small enough that a library is not worth it. */
  function fft(re: Float32Array, im: Float32Array) {
    const n = re.length;
    for (let i = 1, j = 0; i < n; i++) {
      let bit = n >> 1;
      for (; j & bit; bit >>= 1) j ^= bit;
      j ^= bit;
      if (i < j) {
        [re[i], re[j]] = [re[j], re[i]];
        [im[i], im[j]] = [im[j], im[i]];
      }
    }
    for (let len = 2; len <= n; len <<= 1) {
      const ang = (-2 * Math.PI) / len;
      const wr = Math.cos(ang);
      const wi = Math.sin(ang);
      for (let i = 0; i < n; i += len) {
        let cr = 1;
        let ci = 0;
        for (let k = 0; k < len / 2; k++) {
          const ur = re[i + k];
          const ui = im[i + k];
          const vr = re[i + k + len / 2] * cr - im[i + k + len / 2] * ci;
          const vi = re[i + k + len / 2] * ci + im[i + k + len / 2] * cr;
          re[i + k] = ur + vr;
          im[i + k] = ui + vi;
          re[i + k + len / 2] = ur - vr;
          im[i + k + len / 2] = ui - vi;
          const nr = cr * wr - ci * wi;
          ci = cr * wi + ci * wr;
          cr = nr;
        }
      }
    }
  }

  function spectrum(samples: number[]): Float32Array {
    const n = 1024;
    const re = new Float32Array(n);
    const im = new Float32Array(n);
    const count = Math.min(n, samples.length);
    for (let i = 0; i < count; i++) {
      // Hann, or a pure tone smears across dozens of bins.
      const w = 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / n);
      re[i] = samples[i] * w;
    }
    fft(re, im);
    const mags = new Float32Array(n / 2);
    for (let i = 0; i < n / 2; i++) mags[i] = Math.hypot(re[i], im[i]) / (n / 4);
    return mags;
  }

  function clamp(v: number, lo: number, hi: number) {
    return v < lo ? lo : v > hi ? hi : v;
  }

  function css(name: string, fallback: string) {
    if (!canvas) return fallback;
    return getComputedStyle(canvas).getPropertyValue(name).trim() || fallback;
  }

  $effect(() => {
    // Touch the inputs so the effect re-runs when a new frame arrives.
    const w = wave;
    const e = env;
    const c = canvas;
    if (!c) return;

    const dpr = window.devicePixelRatio || 1;
    const cssW = c.clientWidth;
    const cssH = c.clientHeight;
    if (c.width !== cssW * dpr || c.height !== cssH * dpr) {
      c.width = cssW * dpr;
      c.height = cssH * dpr;
    }
    const ctx = c.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssW, cssH);

    const accent = css("--accent", "#5eead4");
    const violet = css("--violet", "#a78bfa");
    const line = css("--line", "#232a36");
    const muted = css("--muted", "#7c8798");

    const gap = 10;
    const specH = Math.round(cssH * 0.5);
    const waveH = Math.round(cssH * 0.26);
    const envH = cssH - specH - waveH - gap * 2;

    // --- spectrum ---------------------------------------------------------
    const mags = spectrum(w);
    // Log frequency axis: the interesting content sits between 50 and 1000 Hz
    // and a linear axis would squeeze it into the leftmost few pixels.
    const minHz = 30;
    const maxHz = 8000;
    const binHz = sampleRate / 1024;
    ctx.beginPath();
    for (let x = 0; x < cssW; x++) {
      const hz = minHz * Math.pow(maxHz / minHz, x / cssW);
      const bin = Math.min(mags.length - 1, Math.round(hz / binHz));
      const db = 20 * Math.log10(Math.max(mags[bin], 1e-7));
      // -90..0 dB across the panel. Clamped to the panel: silence maps to
      // -140 dB, which unclamped would put the trace 55% of a panel-height
      // BELOW the panel, drawing over the waveform and envelope beneath it.
      const y = clamp(specH - ((db + 90) / 90) * specH, 0, specH);
      if (x === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.lineTo(cssW, specH);
    ctx.lineTo(0, specH);
    ctx.closePath();
    const grad = ctx.createLinearGradient(0, 0, 0, specH);
    grad.addColorStop(0, accent + "66");
    grad.addColorStop(1, accent + "08");
    ctx.fillStyle = grad;
    ctx.fill();
    ctx.strokeStyle = accent;
    ctx.lineWidth = 1;
    ctx.stroke();

    // Octave gridlines, so the axis is readable.
    ctx.strokeStyle = line;
    ctx.fillStyle = muted;
    ctx.font = "9px ui-monospace, monospace";
    for (const hz of [100, 1000]) {
      const x = (Math.log(hz / minHz) / Math.log(maxHz / minHz)) * cssW;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, specH);
      ctx.stroke();
      ctx.fillText(hz >= 1000 ? `${hz / 1000}k` : `${hz}`, x + 3, specH - 4);
    }

    // --- waveform ---------------------------------------------------------
    const wy = specH + gap;
    ctx.strokeStyle = line;
    ctx.beginPath();
    ctx.moveTo(0, wy + waveH / 2);
    ctx.lineTo(cssW, wy + waveH / 2);
    ctx.stroke();

    ctx.beginPath();
    ctx.strokeStyle = violet;
    ctx.lineWidth = 1;
    for (let x = 0; x < cssW; x++) {
      const i = Math.floor((x / cssW) * w.length);
      // A hot signal must not draw outside its lane either.
      const y = clamp(wy + waveH / 2 - (w[i] ?? 0) * (waveH / 2) * 0.9, wy, wy + waveH);
      if (x === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();

    // --- beat envelope ----------------------------------------------------
    const ey = wy + waveH + gap;
    let peak = 0;
    for (const v of e) peak = Math.max(peak, v);
    // Below this there is no signal, so there is no envelope shape to show --
    // normalising against a near-zero peak would turn float noise into a
    // full-scale trace.
    const silent = peak < 1e-4;
    if (silent) peak = 1;
    ctx.beginPath();
    for (let x = 0; x < cssW; x++) {
      const i = Math.floor((x / cssW) * e.length);
      // Normalised: the shape of the pulse matters here, not its level.
      const y = clamp(ey + envH - ((e[i] ?? 0) / peak) * envH, ey, ey + envH);
      if (x === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.lineTo(cssW, ey + envH);
    ctx.lineTo(0, ey + envH);
    ctx.closePath();
    ctx.fillStyle = silent ? line : violet + "33";
    ctx.fill();

    if (!playing) {
      ctx.fillStyle = muted;
      ctx.font = "11px ui-sans-serif, system-ui";
      ctx.fillText("stopped", 8, 14);
    }
  });
</script>

<canvas bind:this={canvas}></canvas>

<style>
  canvas {
    width: 100%;
    height: 100%;
    display: block;
    border-radius: 8px;
    background: var(--sunken);
    border: 1px solid var(--line);
  }
</style>
