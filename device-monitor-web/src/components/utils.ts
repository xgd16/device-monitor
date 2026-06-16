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

export function thermalSensorLabel(name: string): { title: string; description: string } {
  const normalized = name.toLowerCase();

  if (normalized.includes('qcom-battery') || normalized.includes('battery')) {
    return { title: '电池温度', description: '电池包 / 充放电区域' };
  }
  if (normalized.includes('wlan') || normalized.includes('wifi')) {
    return { title: '无线网络', description: 'Wi-Fi / WLAN 模块' };
  }
  if (normalized.includes('camera')) {
    return { title: '相机区域', description: '摄像头传感器附近' };
  }
  if (normalized.includes('modem')) {
    return { title: '基带通信', description: '蜂窝网络 / 调制解调器' };
  }
  if (normalized.includes('gpu-top')) {
    return { title: 'GPU 上部', description: '图形处理器热点' };
  }
  if (normalized.includes('gpu-bottom')) {
    return { title: 'GPU 下部', description: '图形处理器背面区域' };
  }
  if (normalized.includes('gpu')) {
    return { title: '图形处理器', description: 'GPU 区域' };
  }
  if (normalized.includes('cpu')) {
    const core = normalized.match(/cpu(\d+)/)?.[1];
    return {
      title: core ? `CPU 核心 ${core}` : '处理器核心',
      description: 'CPU 单核心温度',
    };
  }
  if (normalized.includes('cluster0')) {
    return { title: '小核心集群', description: 'CPU 低功耗核心组' };
  }
  if (normalized.includes('cluster1')) {
    return { title: '大核心集群', description: 'CPU 高性能核心组' };
  }
  if (normalized.includes('mem')) {
    return { title: '内存区域', description: 'RAM / 内存控制器附近' };
  }
  if (normalized.includes('video')) {
    return { title: '视频编解码', description: '视频处理单元' };
  }
  if (normalized.includes('hvx')) {
    return { title: 'DSP 向量单元', description: 'Hexagon HVX 协处理器' };
  }
  if (normalized.includes('pm')) {
    return { title: '电源管理', description: 'PMIC 电源管理芯片' };
  }
  if (normalized.includes('aoss')) {
    const id = normalized.match(/aoss(\d+)/)?.[1];
    return {
      title: id ? `低功耗子系统 ${id}` : '低功耗子系统',
      description: 'Always-On 子系统',
    };
  }

  return { title: name.replace(/-thermal$/i, ''), description: '系统温度传感器' };
}
