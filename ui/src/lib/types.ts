export interface Field {
  id: string;
  name: string;
  value: unknown;
  unit?: string;
  text?: string;
}

export interface Packet {
  seq: number;
  ts: number;
  prio: number;
  pgn: number;
  src: number;
  dst: number;
  len: number;
  data: string;
  diff?: number[];
  frames: number;
  tx?: boolean;
  name: string;
  summary: string;
  fields?: Field[];
}

export interface IsoName {
  unique_number: number;
  manufacturer_code: number;
  device_instance_lower: number;
  device_instance_upper: number;
  device_function: number;
  device_class: number;
  system_instance: number;
  industry_group: number;
  arbitrary_address_capable: boolean;
}

export interface ProductInfo {
  nmea2000_version: number | null;
  product_code: number | null;
  model_id: string;
  software_version: string;
  model_version: string;
  serial_code: string;
  certification_level: number | null;
  load_equivalency: number | null;
}

export interface ObservedPgn {
  count: number;
  last_us: number;
  last_data: number[];
  destination: number;
  keys: string[];
}

export interface Device {
  key: string;
  name_raw: number;
  name: IsoName | null;
  address: number | null;
  address_history: [number, number][];
  first_seen_us: number;
  last_seen_us: number;
  online: boolean;
  product: ProductInfo | null;
  config: { installation_description1: string; installation_description2: string; manufacturer_information: string } | null;
  tx_pgns: number[] | null;
  rx_pgns: number[] | null;
  heartbeat_interval_ms: number | null;
  observed: Record<string, ObservedPgn>;
  frames: number;
  labels: { instance: number; data_source: number; indicator: number; hardware_channel: number | null; label: string }[];
  manufacturer: string | null;
  device_class_text: string | null;
  device_function_text: string | null;
  model: string | null;
  queried: boolean;
}

export interface Second {
  t: number;
  frames: number;
  messages: number;
  errors: number;
  bits: number;
  load_pct: number;
}

export interface Metrics {
  frames_per_sec: number;
  messages_per_sec: number;
  errors_per_sec: number;
  load_pct: number;
  peak_load_pct: number;
  total_frames: number;
  total_messages: number;
  total_errors: number;
  dropped: number;
  fp_timeouts: number;
  fp_orphans: number;
  tp_aborts: number;
  history: Second[];
  per_source: [number, number, number][];
  per_pgn: [number, number, number][];
}

export interface Bank {
  instance: number;
  source: number;
  device_key: string | null;
  states: number[];
  currents: (number | null)[];
  labels: (string | null)[];
  last_us: number;
}

export interface Alert {
  seq: number;
  ts_us: number;
  severity: "info" | "warning" | "error";
  kind: string;
  message: string;
}

export interface Caps {
  raw_frames: boolean;
  can_transmit: boolean;
  tx_messages: boolean;
  error_frames: boolean;
  hw_timestamps: boolean;
}

export interface Units {
  depth: string;
  distance: string;
  temperature: string;
  speed: string;
  volume: string;
  pressure: string;
}

export interface Status {
  version: string;
  units?: Units;
  interface: { uri: string; name: string } | null;
  interface_status: { state: string; detail?: string; reason?: string };
  caps: Caps;
  own_name: string;
  own_address: number | null;
  claim: "none" | "claiming" | "claimed" | "gateway_managed" | "cannot_claim";
  tx_mode: "passive" | "query";
  allow_tx: boolean;
  armed: boolean;
  armed_remaining_s: number | null;
  expert: boolean;
  offline: string | null;
  capture: { file: string; format: string; frames: number; messages: number; bytes: number; seconds: number } | null;
  pgn_db_version: string;
  schemas: number;
  ring: [number, number];
  last_seq: number;
  /** "ios", "android", "macos", "windows", "linux" */
  platform?: string;
  /** The host app can present a share sheet (iPhone / iPad). */
  can_share?: boolean;
}

export interface Finding {
  severity: "info" | "warning" | "error";
  kind: string;
  message: string;
  devices: string[];
  pgn: number | null;
  suggestion: string | null;
  target?: string;
  key?: string;
  new_instance?: number;
}

export interface InterfaceKind {
  scheme: string;
  label: string;
  example: string;
  available: boolean;
  note: string;
}
