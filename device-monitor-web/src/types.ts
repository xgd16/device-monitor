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
