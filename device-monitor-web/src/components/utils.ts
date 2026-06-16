export function fmtUptime(s: number): string {
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d > 0) return `${d}天${h}时${m}分`;
  if (h > 0) return `${h}时${m}分`;
  return `${m}分`;
}

export function fmtBytes(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1048576) return `${(b / 1024).toFixed(1)} KB`;
  if (b < 1073741824) return `${(b / 1048576).toFixed(1)} MB`;
  return `${(b / 1073741824).toFixed(2)} GB`;
}

export function fmtSpeed(bps: number): string {
  if (bps < 1024) return `${bps.toFixed(0)} B/s`;
  if (bps < 1048576) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${(bps / 1048576).toFixed(2)} MB/s`;
}

export function statusLabel(s: string): string {
  if (s.includes('run')) return '运行';
  if (s.includes('sleep')) return '休眠';
  if (s.includes('stop')) return '停止';
  if (s.includes('zombie')) return '僵尸';
  return s;
}

export function statusColor(s: string): 'success' | 'accent' | 'warning' | 'danger' | 'default' {
  if (s.includes('run')) return 'success';
  if (s.includes('sleep')) return 'accent';
  if (s.includes('stop')) return 'warning';
  if (s.includes('zombie')) return 'danger';
  return 'default';
}

export function tempColor(t: number): string {
  if (t > 70) return 'danger';
  if (t > 55) return 'warning';
  return 'success';
}

export function percentColor(v: number): string {
  if (v > 80) return 'danger';
  if (v > 50) return 'warning';
  return 'success';
}

export function batteryStatusLabel(status: string): string {
  switch (status) {
    case 'Charging':
      return '充电中';
    case 'Discharging':
      return '放电中';
    case 'Not charging':
      return '未充电';
    case 'Full':
      return '已充满';
    default:
      return status;
  }
}
