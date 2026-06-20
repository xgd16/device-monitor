import axios from 'axios';
import type { SystemOverview, ProcessInfo, WifiInfo, BluetoothInfo, AlertItem, HistorySeries, DatabaseStats, FileListResponse, FileReadResponse } from '../types';

const api = axios.create({
  baseURL: '/api',
  timeout: 10000,
});

export const fetchOverview = () => api.get<{ data: SystemOverview }>('/system/overview').then(r => r.data.data);
export const fetchCpu = () => api.get('/cpu').then(r => r.data.data);
export const fetchMemory = () => api.get('/memory').then(r => r.data.data);
export const fetchDisk = () => api.get('/disk').then(r => r.data.data);
export const fetchThermal = () => api.get('/thermal').then(r => r.data.data);
export const fetchBattery = () => api.get('/battery').then(r => r.data.data);
export const fetchNetwork = () => api.get('/network').then(r => r.data.data);
export const fetchWifi = () => api.get('/network/wifi').then(r => r.data.data as WifiInfo);
export const fetchBluetooth = () => api.get('/network/bluetooth').then(r => r.data.data as BluetoothInfo);
export const fetchProcesses = () => api.get('/process').then(r => r.data.data as ProcessInfo[]);
export const fetchProcessDetail = (pid: number) => api.get(`/process/${pid}`).then(r => r.data.data);
export const killProcess = (pid: number, signal: string = 'TERM') =>
  api.post(`/process/${pid}/kill`, { signal }).then(r => r.data);
export const fetchLogs = (params?: { lines?: number; keyword?: string; level?: string }) =>
  api.get('/logs', { params }).then(r => r.data.data);
export const fetchAlerts = () => api.get('/alerts').then(r => r.data.data as AlertItem[]);
export const fetchHistoryMetrics = (range: string = '24h', maxPoints = 500) =>
  api.get('/history/metrics', { params: { range, max_points: maxPoints } }).then(r => r.data.data as HistorySeries);
export const fetchDatabaseStats = () => api.get('/database/stats').then(r => r.data.data as DatabaseStats);
export const fetchAlertConfig = () => api.get('/alerts/config').then(r => r.data.data);
export const updateAlertConfig = (config: any) => api.put('/alerts/config', config).then(r => r.data.data);
export const fetchHardware = () => api.get('/hardware').then(r => r.data.data);
export const setFlashlight = (led: string, on: boolean) => api.post('/hardware/flashlight', { led, on }).then(r => r.data);
export const setBrightness = (percent: number) => api.post('/hardware/brightness', { percent }).then(r => r.data);
export const setScreenPower = (on: boolean) => api.post('/hardware/screen', { on }).then(r => r.data);
export const vibrate = (duration_ms: number) => api.post('/hardware/vibrate', { duration_ms }).then(r => r.data);
export const vibratePattern = (segments: [number, number, number][], repeat: boolean) =>
  api.post('/hardware/vibrate/pattern', { segments: segments.map(([d, s, w]) => ({ duration_ms: d, strong_pct: s, weak_pct: w })), repeat }).then(r => r.data);
export const vibrateStop = () => api.post('/hardware/vibrate/stop').then(r => r.data);
export const clearMemory = () => api.post('/hardware/clear-memory').then(r => r.data);
export const setStatusLed = (on?: boolean, percent?: number) =>
  api.post('/hardware/status-led', { on, percent }).then(r => r.data);
export const setCpuStatusLedLink = (enabled: boolean) =>
  api.post('/hardware/cpu-status-led-link', { enabled }).then(r => r.data);
export const setChargeCurrent = (microamps: number) =>
  api.post('/hardware/charge-current', { microamps }).then(r => r.data);
export const setChargeMode = (powerOnly: boolean) =>
  api.post('/hardware/charge-mode', { power_only: powerOnly }).then(r => r.data);
export const setGpuMaxFreq = (max_mhz: number) =>
  api.post('/hardware/gpu-max-freq', { max_mhz }).then(r => r.data);
export const setWifiPowerSave = (enabled: boolean) =>
  api.post('/hardware/wifi-power-save', { enabled }).then(r => r.data);
export const setSpeakerVolume = (percent: number) =>
  api.post('/hardware/speaker/volume', { percent }).then(r => r.data);
export const setSpeakerMute = (muted: boolean) =>
  api.post('/hardware/speaker/mute', { muted }).then(r => r.data);
export const playSpeakerTest = () =>
  api.post('/hardware/speaker/test').then(r => r.data);
export const updateMihomoSubscription = () =>
  api.post('/mihomo/subscription/update', {}, { timeout: 120000 }).then(r => r.data.data);

// ── 文件管理 ──

export const listFiles = (path: string = '/') =>
  api.get('/files/list', { params: { path } }).then(r => r.data.data as FileListResponse);

export const statFile = (path: string) =>
  api.get('/files/stat', { params: { path } }).then(r => r.data.data);

export const readFile = (path: string, offset = 0, limit = 256 * 1024) =>
  api.get('/files/read', { params: { path, offset, limit } }).then(r => r.data.data as FileReadResponse);

export const writeFile = (path: string, content: string, create = false) =>
  api.put('/files/write', { path, content, create }).then(r => r.data);

export const uploadFiles = (dirPath: string, files: File[]) => {
  const form = new FormData();
  form.append('path', dirPath);
  for (const f of files) {
    form.append('file', f, f.name);
  }
  return api.post('/files/upload', form, {
    headers: { 'Content-Type': 'multipart/form-data' },
    timeout: 120000,
  }).then(r => r.data);
};

export const downloadUrl = (path: string) =>
  `/api/files/download?path=${encodeURIComponent(path)}`;

export const previewUrl = (path: string) =>
  `/api/files/download?path=${encodeURIComponent(path)}&inline=1`;

export const mkdir = (path: string) =>
  api.post('/files/mkdir', { path }).then(r => r.data);

export const renameFile = (from: string, to: string) =>
  api.post('/files/rename', { from, to }).then(r => r.data);

export const moveFile = (from: string, to: string) =>
  api.post('/files/move', { from, to }).then(r => r.data);

export const copyFile = (from: string, to: string) =>
  api.post('/files/copy', { from, to }).then(r => r.data);

export const deleteFile = (path: string, recursive = false) =>
  api.delete('/files/delete', { params: { path, recursive } }).then(r => r.data);

export type ArchiveFormat = 'zip' | '7z' | 'rar';

export const compressFiles = (paths: string[], output: string, format: ArchiveFormat) =>
  api.post('/files/compress', { paths, output, format }, { timeout: 300000 }).then(r => r.data);

export const extractFiles = (path: string, dest: string, overwrite = false) =>
  api.post('/files/extract', { path, dest, overwrite }, { timeout: 300000 }).then(r => r.data);
