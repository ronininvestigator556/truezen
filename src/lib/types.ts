/** Mirrors `truezen_engine::layer::LayerConfig`. */
export type LayerKind = "binaural" | "monaural" | "isochronic" | "noise" | "file";
export type Waveform = "sine" | "triangle" | "square" | "saw";
export type NoiseColor = "white" | "pink" | "brown";
export type FilterMode = "low_pass" | "band_pass" | "high_pass";

export interface LayerConfig {
  name: string;
  kind: LayerKind;
  enabled: boolean;
  carrier_hz: number;
  beat_hz: number;
  gain: number;
  pan: number;
  waveform: Waveform;
  duty: number;
  ramp_ms: number;
  depth: number;
  am_rate_hz: number;
  am_depth: number;
  noise_color: NoiseColor;
  filter_mode: FilterMode;
  filter_cutoff_hz: number;
  filter_q: number;
  lfo_rate_hz: number;
  lfo_depth: number;
  file_path: string | null;
  loop_file: boolean;
  ducks_others: boolean;
  duck_depth: number;
}

export type Curve = "hold" | "linear" | "exponential" | "smoothstep";
export type LayerParam =
  | "carrier"
  | "beat"
  | "gain"
  | "pan"
  | "filter_cutoff"
  | "duty"
  | "ramp_ms"
  | "depth";

export type ParamTarget =
  | { kind: "master_gain" }
  | { kind: "layer"; index: number; param: LayerParam };

export interface Track {
  target: ParamTarget;
  points: { at_s: number; value: number; curve: Curve }[];
}

export interface Timeline {
  tracks: Track[];
  duration_s: number | null;
  fade_out_s: number;
}

export interface Preset {
  schema_version: number;
  id: string;
  name: string;
  goal: string;
  description: string;
  requires_headphones: boolean;
  master_gain: number;
  layers: LayerConfig[];
  timeline: Timeline | null;
}

export type PresetSource = "factory" | "user" | "override";

export interface PresetSummary {
  id: string;
  name: string;
  goal: string;
  description: string;
  requiresHeadphones: boolean;
  durationS: number;
  layerCount: number;
  source: PresetSource;
}

/** The live preset, whether it has unsaved edits, and where the library lives. */
export interface Editing {
  preset: Preset | null;
  dirty: boolean;
  source: PresetSource | null;
  libraryDir: string;
}

export interface Meters {
  peakL: number;
  peakR: number;
  rmsL: number;
  rmsR: number;
  positionS: number;
  durationS: number;
  playing: boolean;
  finished: boolean;
  beatHz: number;
  carrierHz: number;
  /** Bit n set means timeline track n is latched to a manual value. */
  latchedTracks: number;
}

export interface Frame {
  meters: Meters;
  wave: number[];
  env: number[];
}

export interface HostStatus {
  deviceName: string | null;
  sampleRate: number;
  channels: number;
  running: boolean;
  binauralCapable: boolean;
  lastError: string | null;
}

export interface Stats {
  callbacks: number;
  overloads: number;
  errors: number;
  droppedCommands: number;
  lastLoadPermille: number;
  maxLoadPermille: number;
}

export interface Health {
  status: HostStatus;
  stats: Stats;
  error: string | null;
}

export interface DeviceInfo {
  id: string;
  name: string;
  isDefault: boolean;
}

/** EEG bands, used to label whatever beat frequency the user has dialed in. */
export const BANDS = [
  { name: "delta", max: 4, hint: "deep sleep" },
  { name: "theta", max: 8, hint: "reverie, deep meditation" },
  { name: "alpha", max: 12, hint: "relaxed, settled" },
  { name: "SMR", max: 15, hint: "calm focus" },
  { name: "beta", max: 30, hint: "alert, analytical" },
  { name: "gamma", max: Infinity, hint: "concentration" },
] as const;

export function bandFor(hz: number) {
  return BANDS.find((b) => hz < b.max) ?? BANDS[BANDS.length - 1];
}
