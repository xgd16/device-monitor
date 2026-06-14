import axios from 'axios';
import type { SystemOverview, ProcessInfo, WifiInfo, BluetoothInfo, AlertItem, HistorySeries, DatabaseStats } from '../types';

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
