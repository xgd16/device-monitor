export interface CpuCore {
  id: number;
  usage: number;
  frequency_mhz: number;
}

export interface CpuInfo {
  overall_usage: number;
  cores: CpuCore[];
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
