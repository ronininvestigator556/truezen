import type { Curve, LayerParam, ParamTarget, Track } from "./types";

/**
 * Curve evaluation, mirroring `truezen_engine::timeline`.
 *
 * This has to match the Rust exactly. If it drifts, the editor draws a line
 * that is not the line you hear, which is worse than having no editor.
 */
export function interpolate(a: number, b: number, u: number, curve: Curve): number {
  switch (curve) {
    case "hold":
      return a;
    case "linear":
      return a + (b - a) * u;
    case "smoothstep":
      return a + (b - a) * (u * u * (3 - 2 * u));
    case "exponential":
      // Geometric interpolation is undefined through zero, which a gain track
      // will hit; the engine falls back to linear there and so must this.
      return a > 0 && b > 0 ? a * Math.pow(b / a, u) : a + (b - a) * u;
  }
}

/** Value of a track at time `t`, clamping outside its defined range. */
export function valueAt(track: Track, t: number): number | null {
  const pts = track.points;
  if (pts.length === 0) return null;
  if (t <= pts[0].at_s) return pts[0].value;
  const last = pts[pts.length - 1];
  if (t >= last.at_s) return last.value;

  const i = pts.findIndex((p) => p.at_s > t);
  if (i <= 0) return last.value;
  const a = pts[i - 1];
  const b = pts[i];
  const span = b.at_s - a.at_s;
  if (span <= 0) return b.value;
  return interpolate(a.value, b.value, (t - a.at_s) / span, b.curve);
}

export interface Range {
  min: number;
  max: number;
  log?: boolean;
  unit: string;
  step: number;
  decimals: number;
  label: string;
}

/** Editable range per parameter. Mirrors the limits the Lab view offers. */
export const PARAM_RANGE: Record<LayerParam, Range> = {
  carrier: { min: 20, max: 2000, log: true, unit: "Hz", step: 0.1, decimals: 2, label: "Carrier" },
  beat: { min: 0, max: 60, unit: "Hz", step: 0.01, decimals: 2, label: "Beat" },
  gain: { min: 0, max: 1, unit: "", step: 0.005, decimals: 3, label: "Level" },
  pan: { min: -1, max: 1, unit: "", step: 0.01, decimals: 2, label: "Pan" },
  filter_cutoff: {
    min: 30, max: 18000, log: true, unit: "Hz", step: 1, decimals: 0, label: "Cutoff",
  },
  duty: { min: 0.05, max: 1, unit: "", step: 0.01, decimals: 2, label: "Duty" },
  ramp_ms: { min: 0.5, max: 100, unit: "ms", step: 0.5, decimals: 1, label: "Ramp" },
  depth: { min: 0, max: 1, unit: "", step: 0.01, decimals: 2, label: "Depth" },
};

export const MASTER_RANGE: Range = {
  min: 0, max: 1, unit: "", step: 0.005, decimals: 3, label: "Master",
};

export function rangeFor(target: ParamTarget): Range {
  return target.kind === "master_gain" ? MASTER_RANGE : PARAM_RANGE[target.param];
}

/** Map a value to 0..1 within an arbitrary window, geometrically where asked. */
export function normaliseIn(v: number, lo: number, hi: number, log?: boolean): number {
  if (log && lo > 0 && hi > lo) {
    const a = Math.log(lo);
    return (Math.log(Math.max(v, lo)) - a) / (Math.log(hi) - a);
  }
  return hi > lo ? (v - lo) / (hi - lo) : 0;
}

/** Inverse of `normaliseIn`. */
export function denormaliseIn(u: number, lo: number, hi: number, log?: boolean): number {
  const c = Math.min(1, Math.max(0, u));
  if (log && lo > 0) return lo * Math.pow(hi / lo, c);
  return lo + c * (hi - lo);
}

/**
 * The value window a timeline lane should show.
 *
 * Not the parameter's full range: a beat track moving between 8 and 10 Hz
 * drawn against 0-60 Hz is a flat line crushed against the floor. Fitting the
 * lane to its own data, with room to drag beyond it, is what makes the shape
 * of a session legible.
 */
export function laneWindow(values: number[], r: Range): { lo: number; hi: number } {
  if (values.length === 0) return { lo: r.min, hi: r.max };
  let lo = Math.min(...values);
  let hi = Math.max(...values);
  const span = hi - lo;
  // A flat track still needs a workable window to drag within.
  const pad = Math.max(span * 0.6, (r.max - r.min) * 0.03);
  lo = Math.max(r.min, lo - pad);
  hi = Math.min(r.max, hi + pad);
  if (hi - lo < 1e-9) hi = Math.min(r.max, lo + (r.max - r.min) * 0.1);
  return { lo, hi };
}

export function quantise(v: number, r: Range): number {
  const q = Math.round(v / r.step) * r.step;
  return Number.parseFloat(Math.min(r.max, Math.max(r.min, q)).toFixed(6));
}

export function clock(s: number): string {
  if (!Number.isFinite(s) || s < 0) s = 0;
  const m = Math.floor(s / 60);
  const sec = Math.floor(s % 60);
  return `${m}:${sec.toString().padStart(2, "0")}`;
}
