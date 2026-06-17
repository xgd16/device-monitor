import { useState, useEffect, useCallback } from 'react';
import type { ReactNode } from 'react';
import { Card, Chip } from '@heroui/react';
import type { SystemOverview, ProcessInfo } from '../types';
import { fetchHardware } from '../api';
import { percentColor, tempColor, fmtChargeUa, chargeSourceLabel } from './utils';

interface SystemStatusCardProps {
  data: SystemOverview;
  processes: ProcessInfo[];
  netSpeed: Record<string, { rx: number; tx: number }>;
}

function fmtSpeed(bps: number) {
  if (bps >= 1048576) return `${(bps / 1048576).toFixed(1)} MB/s`;
  if (bps >= 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${bps.toFixed(0)} B/s`;
}

function fmtBytes(bytes: number) {
  if (bytes >= 1073741824) return `${(bytes / 1073741824).toFixed(1)}GB`;
  if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)}MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${bytes}B`;
}

function shortProxyName(name: string) {
  return name
    .replace(/网址[:：]\S+/g, '')
    .replace(/\s+/g, ' ')
    .trim() || '未知';
}

function fmtUa(ua: number) {
  return fmtChargeUa(ua);
}

function fmtUptime(sec: number) {
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  if (h >= 24) return `${Math.floor(h / 24)}天 ${h % 24}时`;
  return `${h}时 ${m}分`;
}

export function SystemStatusCard({ data, processes, netSpeed }: SystemStatusCardProps) {
  const [hw, setHw] = useState<{
    screen_on: boolean;
    charging: {
      current_max_ua: number;
      target_current_max_ua: number;
      current_now_ua: number;
      power_w: number;
      charger_online: boolean;
      charge_source: string;
      usb_type: string;
      charge_mode: string;
    };
    wifi_power_save: { enabled: boolean };
  } | null>(null);

  const refresh = useCallback(() => {
    fetchHardware()
      .then(d => setHw({
        screen_on: d.screen_on,
        charging: d.charging,
        wifi_power_save: d.wifi_power_save,
      }))
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 5000);
    return () => clearInterval(t);
  }, [refresh]);

  const maxTemp = Math.max(...data.thermal.map(t => t.temp_celsius), 0);
  const wlan = data.network.find(n => n.name === 'wlan0');
  const wlanSpeed = netSpeed.wlan0;
  const topMem = [...processes].sort((a, b) => b.memory_mb - a.memory_mb).slice(0, 4);
  const { battery } = data;

  const rows: { label: string; value: ReactNode }[] = [
    {
      label: '运行时间',
      value: <span className="font-mono text-sm">{fmtUptime(data.uptime)}</span>,
    },
    {
      label: '负载',
      value: (
        <span className="font-mono text-sm">
          {data.load_avg.map(v => v.toFixed(2)).join(' / ')}
        </span>
      ),
    },
    {
      label: '电池',
      value: (
        <span className="font-mono text-sm flex items-center gap-2">
          <span style={{ color: `var(--${percentColor(battery.capacity)})` }}>{battery.capacity}%</span>
          <span className="opacity-40 text-[10px]">{battery.status}</span>
          {battery.power_w > 0 && <span className="opacity-40 text-[10px]">{battery.power_w.toFixed(1)}W</span>}
        </span>
      ),
    },
    {
      label: '最高温度',
      value: (
        <span className="font-mono text-sm" style={{ color: `var(--${tempColor(maxTemp)})` }}>
          {maxTemp.toFixed(1)}°C
        </span>
      ),
    },
  ];

  if (wlan) {
    rows.push({
      label: 'WiFi',
      value: (
        <span className="font-mono text-[11px] flex flex-col items-end gap-0.5">
          <span className={wlan.is_up ? 'text-success' : 'opacity-40'}>{wlan.is_up ? '已连接' : '未连接'}</span>
          {wlanSpeed && (
            <span className="opacity-40 text-[10px]">
              ↓{fmtSpeed(wlanSpeed.rx)} · ↑{fmtSpeed(wlanSpeed.tx)}
            </span>
          )}
        </span>
      ),
    });
  }

  rows.push({
    label: 'VPN',
    value: data.mihomo?.available ? (
      <span className="font-mono text-[11px] flex flex-col items-end gap-0.5">
        <span className="text-success">
          {data.mihomo.tun_enabled ? 'TUN' : '代理'} · {shortProxyName(data.mihomo.active_proxy)}
        </span>
        <span className="opacity-40 text-[10px]">
          {data.mihomo.mode || 'rule'} · {data.mihomo.connection_count} 连接 · ↓{fmtBytes(data.mihomo.download_total)} ↑{fmtBytes(data.mihomo.upload_total)}
        </span>
      </span>
    ) : (
      <span className="font-mono text-[11px] opacity-40">未连接</span>
    ),
  });

  if (hw) {
    rows.push(
      {
        label: '屏幕',
        value: <Chip size="sm" color={hw.screen_on ? 'success' : 'default'} variant="secondary">{hw.screen_on ? '亮屏' : '息屏'}</Chip>,
      },
      {
        label: '充电',
        value: (
          <span className="font-mono text-[11px] flex items-center gap-2">
            <Chip size="sm" color={hw.charging.charger_online ? 'success' : 'default'} variant="secondary">
              {chargeSourceLabel(hw.charging.charge_source)}
            </Chip>
            {hw.charging.charge_mode === 'power_only' && (
              <Chip size="sm" color="warning" variant="secondary">仅供电</Chip>
            )}
            <span className="opacity-40">
              {fmtUa(hw.charging.target_current_max_ua || hw.charging.current_max_ua)}
            </span>
            {hw.charging.charger_online && hw.charging.power_w > 0 && (
              <span className="opacity-40">{hw.charging.power_w.toFixed(1)}W</span>
            )}
          </span>
        ),
      },
      {
        label: 'WiFi 省电',
        value: (
          <Chip size="sm" color={hw.wifi_power_save.enabled ? 'warning' : 'success'} variant="secondary">
            {hw.wifi_power_save.enabled ? '开启' : '关闭'}
          </Chip>
        ),
      },
    );
  }

  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-3">
      <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">系统状态</span>

      <div className="flex flex-col gap-2">
        {rows.map(row => (
          <div key={row.label} className="flex items-center justify-between gap-3 py-1 border-b border-default-100 last:border-0">
            <span className="font-mono text-[10px] opacity-40 shrink-0">{row.label}</span>
            {row.value}
          </div>
        ))}
      </div>

      {topMem.length > 0 && (
        <div className="flex flex-col gap-1.5 pt-1">
          <span className="text-[9px] font-mono uppercase tracking-widest opacity-30">内存占用 Top</span>
          {topMem.map(p => (
            <div key={p.pid} className="flex items-center gap-2 font-mono text-[10px]">
              <span className="flex-1 truncate opacity-50">{p.name}</span>
              <span className="opacity-30">{p.pid}</span>
              <span>{p.memory_mb >= 1024 ? `${(p.memory_mb / 1024).toFixed(1)}G` : `${p.memory_mb}M`}</span>
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}
