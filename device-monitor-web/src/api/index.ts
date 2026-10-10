import axios from 'axios';
import type {
  SystemOverview,
  ProcessInfo,
  WifiInfo,
  BluetoothInfo,
  AlertItem,
  HistorySeries,
  DatabaseStats,
  FileListResponse,
  FileReadResponse,
  NfcStatus,
  NfcTag,
  NfcReadResult,
  NfcWriteResult,
  NfcRawResult,
  NfcEvent,
  NfcEmulStatus,
} from '../types';

const api = axios.create({
  baseURL: '/api',
  timeout: 10000,
});

export const fetchOverview = () =>
  api.get<{ data: SystemOverview }>('/system/overview').then((r) => r.data.data);

// ── 界面刷新间隔 ──

export interface RefreshConfig {
  /** 当前生效的采集心跳（秒），WS 推送与 DRM 物理屏重绘跟随 */
  refresh_secs: number;
  /** 服务端允许的取值 */
  choices: number[];
}

export const fetchRefresh = () =>
  api.get('/system/refresh').then((r) => r.data.data as RefreshConfig);
export const setRefresh = (secs: number) =>
  api.post('/system/refresh', { secs }).then((r) => r.data.data as { refresh_secs: number });
export const fetchCpu = () => api.get('/cpu').then((r) => r.data.data);
export const fetchMemory = () => api.get('/memory').then((r) => r.data.data);
export const fetchDisk = () => api.get('/disk').then((r) => r.data.data);
export const fetchThermal = () => api.get('/thermal').then((r) => r.data.data);
export const fetchBattery = () => api.get('/battery').then((r) => r.data.data);
export const fetchNetwork = () => api.get('/network').then((r) => r.data.data);
export const fetchWifi = () => api.get('/network/wifi').then((r) => r.data.data as WifiInfo);
export const fetchBluetooth = () =>
  api.get('/network/bluetooth').then((r) => r.data.data as BluetoothInfo);
export const fetchProcesses = () => api.get('/process').then((r) => r.data.data as ProcessInfo[]);
export const fetchProcessDetail = (pid: number) =>
  api.get(`/process/${pid}`).then((r) => r.data.data);
export const killProcess = (pid: number, signal: string = 'TERM') =>
  api.post(`/process/${pid}/kill`, { signal }).then((r) => r.data);
export const fetchLogs = (params?: { lines?: number; keyword?: string; level?: string }) =>
  api.get('/logs', { params }).then((r) => r.data.data);
export const fetchAlerts = () => api.get('/alerts').then((r) => r.data.data as AlertItem[]);
export const fetchHistoryMetrics = (range: string = '24h', maxPoints = 500) =>
  api
    .get('/history/metrics', { params: { range, max_points: maxPoints } })
    .then((r) => r.data.data as HistorySeries);
export const fetchDatabaseStats = () =>
  api.get('/database/stats').then((r) => r.data.data as DatabaseStats);
export const fetchAlertConfig = () => api.get('/alerts/config').then((r) => r.data.data);
export const updateAlertConfig = (config: any) =>
  api.put('/alerts/config', config).then((r) => r.data.data);
export const fetchHardware = () => api.get('/hardware').then((r) => r.data.data);
export const setFlashlight = (led: string, on: boolean) =>
  api.post('/hardware/flashlight', { led, on }).then((r) => r.data);
export const setBrightness = (percent: number) =>
  api.post('/hardware/brightness', { percent }).then((r) => r.data);
export const setScreenPower = (on: boolean) =>
  api.post('/hardware/screen', { on }).then((r) => r.data);
export const vibrate = (duration_ms: number) =>
  api.post('/hardware/vibrate', { duration_ms }).then((r) => r.data);
export const vibratePattern = (segments: [number, number, number][], repeat: boolean) =>
  api
    .post('/hardware/vibrate/pattern', {
      segments: segments.map(([d, s, w]) => ({ duration_ms: d, strong_pct: s, weak_pct: w })),
      repeat,
    })
    .then((r) => r.data);
export const vibrateStop = () => api.post('/hardware/vibrate/stop').then((r) => r.data);
export const clearMemory = () => api.post('/hardware/clear-memory').then((r) => r.data);
export const setStatusLed = (on?: boolean, percent?: number) =>
  api.post('/hardware/status-led', { on, percent }).then((r) => r.data);
export const setCpuStatusLedLink = (enabled: boolean) =>
  api.post('/hardware/cpu-status-led-link', { enabled }).then((r) => r.data);
export const setChargeCurrent = (microamps: number) =>
  api.post('/hardware/charge-current', { microamps }).then((r) => r.data);
export const setChargeMode = (powerOnly: boolean) =>
  api.post('/hardware/charge-mode', { power_only: powerOnly }).then((r) => r.data);
export const setGpuMaxFreq = (max_mhz: number) =>
  api.post('/hardware/gpu-max-freq', { max_mhz }).then((r) => r.data);
export const setWifiPowerSave = (enabled: boolean) =>
  api.post('/hardware/wifi-power-save', { enabled }).then((r) => r.data);
export const setSpeakerVolume = (percent: number) =>
  api.post('/hardware/speaker/volume', { percent }).then((r) => r.data);
export const setSpeakerMute = (muted: boolean) =>
  api.post('/hardware/speaker/mute', { muted }).then((r) => r.data);
export const playSpeakerTest = () => api.post('/hardware/speaker/test').then((r) => r.data);
export const updateMihomoSubscription = () =>
  api.post('/mihomo/subscription/update', {}, { timeout: 120000 }).then((r) => r.data.data);

// ── 文件管理 ──

export const listFiles = (path: string = '/') =>
  api.get('/files/list', { params: { path } }).then((r) => r.data.data as FileListResponse);

export const statFile = (path: string) =>
  api.get('/files/stat', { params: { path } }).then((r) => r.data.data);

export const readFile = (path: string, offset = 0, limit = 256 * 1024) =>
  api
    .get('/files/read', { params: { path, offset, limit } })
    .then((r) => r.data.data as FileReadResponse);

export const writeFile = (path: string, content: string, create = false) =>
  api.put('/files/write', { path, content, create }).then((r) => r.data);

export const uploadFiles = (dirPath: string, files: File[]) => {
  const form = new FormData();
  form.append('path', dirPath);
  for (const f of files) {
    form.append('file', f, f.name);
  }
  return api
    .post('/files/upload', form, {
      headers: { 'Content-Type': 'multipart/form-data' },
      timeout: 120000,
    })
    .then((r) => r.data);
};

export const downloadUrl = (path: string) => `/api/files/download?path=${encodeURIComponent(path)}`;

export const previewUrl = (path: string) =>
  `/api/files/download?path=${encodeURIComponent(path)}&inline=1`;

export const mkdir = (path: string) => api.post('/files/mkdir', { path }).then((r) => r.data);

export const renameFile = (from: string, to: string) =>
  api.post('/files/rename', { from, to }).then((r) => r.data);

export const moveFile = (from: string, to: string) =>
  api.post('/files/move', { from, to }).then((r) => r.data);

export const copyFile = (from: string, to: string) =>
  api.post('/files/copy', { from, to }).then((r) => r.data);

export const deleteFile = (path: string, recursive = false) =>
  api.delete('/files/delete', { params: { path, recursive } }).then((r) => r.data);

export type ArchiveFormat = 'zip' | '7z' | 'rar';

export const compressFiles = (paths: string[], output: string, format: ArchiveFormat) =>
  api.post('/files/compress', { paths, output, format }, { timeout: 300000 }).then((r) => r.data);

export const extractFiles = (path: string, dest: string, overwrite = false) =>
  api.post('/files/extract', { path, dest, overwrite }, { timeout: 300000 }).then((r) => r.data);

// ── NFC ──

export const fetchNfcStatus = () => api.get('/nfc/status').then((r) => r.data.data as NfcStatus);

export const setNfcPower = (up: boolean) =>
  api.post('/nfc/power', { up }).then((r) => r.data.data as { powered: boolean });

/** action=start 时 protocols 缺省表示全选 */
export const setNfcPoll = (action: 'start' | 'stop', protocols?: string[]) =>
  api
    .post('/nfc/poll', { action, protocols })
    .then((r) => r.data.data as { polling: boolean; protocols: string[] });

export const setNfcListen = (action: 'start' | 'stop') =>
  api.post('/nfc/listen', { action }).then((r) => r.data.data as { listening: boolean });

export const fetchNfcTags = () => api.get('/nfc/tags').then((r) => r.data.data as NfcTag[]);

export const readNfcTag = (params: {
  target_index: number;
  protocol: string;
  mode: 'ndef' | 'pages';
  start_page?: number;
  page_count?: number;
}) => api.post('/nfc/read', params, { timeout: 30000 }).then((r) => r.data.data as NfcReadResult);

export const writeNfcTag = (params: {
  target_index: number;
  protocol: string;
  kind: 'text' | 'uri' | 'raw';
  value: string;
  lang?: string;
  raw_hex?: string;
}) => api.post('/nfc/write', params, { timeout: 60000 }).then((r) => r.data.data as NfcWriteResult);

export const rawNfcExchange = (params: {
  target_index: number;
  protocol: string;
  tx_hex: string;
  timeout_ms?: number;
}) => api.post('/nfc/raw', params, { timeout: 30000 }).then((r) => r.data.data as NfcRawResult);

export const fetchNfcEvents = (limit = 50) =>
  api.get('/nfc/events', { params: { limit } }).then((r) => r.data.data as NfcEvent[]);

export const fetchNfcEmulate = () =>
  api.get('/nfc/emulate').then((r) => r.data.data as NfcEmulStatus);

export const setNfcEmulate = (params: {
  action: 'start' | 'stop';
  uid?: string;
  kind?: string;
  value?: string;
  lang?: string;
  raw_hex?: string;
}) =>
  api.post('/nfc/emulate', params, { timeout: 15000 }).then((r) => r.data.data as NfcEmulStatus);
