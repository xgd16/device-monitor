export interface CpuCore {
  id: number;
  usage: number;
  frequency_mhz: number;
}

export interface CpuInfo {
  overall_usage: number;
  cores: CpuCore[];
}

export interface CoreGovernor {
  core_id: number;
  governor: string;
}

export interface CpuGovernor {
  current: string;
  available: string[];
  per_core: CoreGovernor[];
}

export interface MemoryInfo {
  total_mb: number;
  used_mb: number;
  free_mb: number;
  available_mb: number;
  swap_total_mb: number;
  swap_used_mb: number;
  usage_percent: number;
}

export interface ThermalZone {
  id: number;
  name: string;
  temp_celsius: number;
}

export interface BatteryInfo {
  capacity: number;
  status: string;
  voltage_v: number;
  current_ma: number;
  power_w: number;
  temp_celsius: number;
  time_left_min: number;
  effective_max_pct?: number;
  display_capacity_pct?: number;
  is_degraded?: boolean;
  at_charge_limit?: boolean;
  /** 学到的实际满容量（mAh），0/undefined = 还没量出来 */
  capacity_mah?: number;
  /** 相对设计容量的健康度（%），0/undefined = 未知 */
  health_percent?: number;
}

export interface NetworkInterface {
  name: string;
  is_up: boolean;
  ip_addresses: string[];
  rx_bytes: number;
  tx_bytes: number;
  rx_packets: number;
  tx_packets: number;
}

export interface MihomoInfo {
  available: boolean;
  controller: string;
  version: string;
  mode: string;
  tun_enabled: boolean;
  active_group: string;
  active_proxy: string;
  proxy_chain: string[];
  connection_count: number;
  upload_total: number;
  download_total: number;
  subscription_last_update: number;
  error: string;
}

export interface ProcessInfo {
  pid: number;
  name: string;
  status: string;
  cpu_usage: number;
  memory_mb: number;
  ppid: number;
  threads: number;
}

export interface WifiInfo {
  connected: boolean;
  ssid: string;
  signal_dbm: number;
  frequency_mhz: number;
  channel: number;
  band: string;
  bitrate: string;
  bssid: string;
}

export interface BluetoothInfo {
  powered: boolean;
  address: string;
  name: string;
  devices: { address: string; name: string; paired: boolean; connected: boolean }[];
}

export interface SystemOverview {
  cpu: CpuInfo;
  memory: MemoryInfo;
  thermal: ThermalZone[];
  battery: BatteryInfo;
  network: NetworkInterface[];
  mihomo: MihomoInfo;
  uptime: number;
  load_avg: number[];
  process_count: number;
  timestamp: number;
}

export interface AlertItem {
  id: number;
  timestamp: number;
  level: string;
  title: string;
  message: string;
}

export interface HistorySeries {
  range: string;
  from: number;
  to: number;
  count: number;
  timestamps: number[];
  cpu_usage: number[];
  memory_percent: number[];
  memory_used_mb: number[];
  load_1: number[];
  load_5: number[];
  load_15: number[];
  battery_capacity: number[];
  battery_power_w: number[];
  thermal_max: number[];
  process_count: number[];
  network_rx_kbps: number[];
  network_tx_kbps: number[];
}

export interface DatabaseStats {
  metrics_count: number;
  alerts_count: number;
  oldest_metric: number | null;
  newest_metric: number | null;
  retention_days: number;
}

export interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  mode: string;
  owner: string;
  group: string;
  modified: number;
  is_symlink: boolean;
}

export interface FileListResponse {
  path: string;
  parent: string | null;
  entries: FileEntry[];
}

export interface FileReadResponse {
  path: string;
  offset: number;
  total_size: number;
  read_size: number;
  is_binary: boolean;
  content: string | null;
  encoding: string;
}

// ── NFC ──

/** 协议名与后端字符串一一对应 */
export type NfcProtocol =
  'jewel' | 'mifare' | 'felica' | 'iso14443a' | 'nfc_dep' | 'iso14443b' | 'iso15693';

/** 单项能力的可用性与不可用原因 */
export interface NfcCapability {
  ok: boolean;
  reason: string;
}

export interface NfcDevice {
  index: number;
  name: string;
  protocols_mask: number;
  protocols: string[];
  powered: boolean;
  rfkill_soft_blocked: boolean;
  rfkill_hard_blocked: boolean;
}

export interface NfcSecureElement {
  index: number;
  se_type: number;
}

export interface NfcCapabilities {
  discover: NfcCapability;
  read: NfcCapability;
  write: NfcCapability;
  emulate: NfcCapability;
  emulate_uid: NfcCapability;
}

export interface NfcStatus {
  available: boolean;
  platform: string;
  message: string;
  device: NfcDevice | null;
  polling: boolean;
  listening: boolean;
  listen_protocols: string[];
  target_count: number;
  secure_elements: NfcSecureElement[];
  capabilities: NfcCapabilities;
  last_error: string | null;
  last_op: string | null;
}

export interface NfcTag {
  index: number;
  protocol: string;
  protocols: string[];
  uid: string;
  atqa: string;
  sak: number | null;
  ats: string | null;
  technology: string;
  best_protocol: string;
  seen_at: string;
  lost: boolean;
}

export interface NfcNdefRecord {
  tnf: number;
  type: string;
  kind: 'text' | 'uri' | 'raw';
  text: string | null;
  uri: string | null;
  payload_hex: string;
}

export interface NfcNdef {
  records: NfcNdefRecord[];
  raw_hex: string;
  size: number;
}

export interface NfcPage {
  page: number;
  hex: string;
  ascii: string;
}

export interface NfcExchange {
  tx: string;
  rx: string;
}

export interface NfcReadResult {
  uid: string;
  technology: string;
  ndef: NfcNdef | null;
  pages: NfcPage[] | null;
  raw_exchanges: NfcExchange[];
  note: string;
}

export interface NfcWriteResult {
  wrote_bytes: number;
  template: string;
  verify: { raw_hex: string; records: number } | null;
}

export interface NfcRawResult {
  tx: string;
  rx: string;
  elapsed_ms: number;
  error: string | null;
}

export interface NfcEvent {
  at: string;
  kind: string;
  text: string;
  detail: unknown;
}

/** UID 卡模拟（用户态直驱芯片）状态 */
export interface NfcEmulStatus {
  running: boolean;
  active: boolean;
  uid: string;
  ndef_hex: string;
  started_at: string | null;
  last_error: string | null;
  events?: NfcEvent[];
  note?: string;
}
