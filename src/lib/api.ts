import { invoke } from "@tauri-apps/api/core";
import type { DeviceInfo, Frame, Health, Preset, PresetSummary, Timeline } from "./types";

export const api = {
  listPresets: () => invoke<PresetSummary[]>("list_presets"),
  goals: () => invoke<string[]>("goals"),
  loadPreset: (id: string) => invoke<Preset>("load_preset", { id }),
  currentPreset: () => invoke<Preset | null>("current_preset"),

  play: () => invoke<void>("play"),
  pause: () => invoke<void>("pause"),
  stop: () => invoke<void>("stop"),
  seek: (seconds: number) => invoke<void>("seek", { seconds }),

  setMasterGain: (gain: number) => invoke<void>("set_master_gain", { gain }),
  setLayerParam: (index: number, param: string, value: number) =>
    invoke<void>("set_layer_param", { index, param, value }),
  setLayerEnabled: (index: number, on: boolean) =>
    invoke<void>("set_layer_enabled", { index, on }),

  updateTimeline: (timeline: Timeline | null) =>
    invoke<void>("update_timeline", { timeline }),

  setTrackLatched: (track: number, latched: boolean) =>
    invoke<void>("set_track_latched", { track, latched }),
  unlatchAll: () => invoke<void>("unlatch_all"),

  poll: () => invoke<Frame>("poll"),
  health: () => invoke<Health>("health"),

  listDevices: () => invoke<DeviceInfo[]>("list_devices"),
  selectDevice: (id: string | null) => invoke<void>("select_device", { id }),
};
