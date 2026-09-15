import { useState, useEffect, useMemo } from 'react';
import { Chip } from '@heroui/react';
import { MeterRow, Panel, StatCell } from './Panel';
import { fetchDisk } from '../api';
import { percentColor } from './utils';

interface DiskInfo {
  device: string;
  mount: string;
  fstype: string;
  total_mb: number;
  used_mb: number;
  available_mb: number;
  usage_percent: number;
  inode_total: number;
  inode_used: number;
  inode_free: number;
  inode_percent: number;
  read_sectors: number;
  write_sectors: number;
  io_ticks_ms: number;
  disk_type: string;
}

function fmtSize(mb: number): string {
  if (mb >= 1024 * 1024) return `${(mb / 1024 / 1024).toFixed(1)} TB`;
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${mb} MB`;
}

function fmtSectors(s: number): string {
  // 1 sector = 512 bytes
  const bytes = s * 512;
  if (bytes >= 1073741824) return `${(bytes / 1073741824).toFixed(1)} GB`;
  if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

function fmtInode(n: number): string {
  if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}K`;
  return `${n}`;
}

export function DiskCard() {
  const [disks, setDisks] = useState<DiskInfo[]>([]);

  useEffect(() => {
    const load = () =>
      fetchDisk()
        .then(setDisks)
        .catch(() => {});
    load();
    const t = setInterval(load, 30000);
    return () => clearInterval(t);
  }, []);

  const summary = useMemo(() => {
    const total = disks.reduce((s, d) => s + d.total_mb, 0);
    const used = disks.reduce((s, d) => s + d.used_mb, 0);
    const avail = disks.reduce((s, d) => s + d.available_mb, 0);
    const readTotal = disks.reduce((s, d) => s + d.read_sectors, 0);
    const writeTotal = disks.reduce((s, d) => s + d.write_sectors, 0);
    return {
      total,
      used,
      avail,
      pct: total > 0 ? (used / total) * 100 : 0,
      readTotal,
      writeTotal,
    };
  }, [disks]);

  if (disks.length === 0) return null;

  return (
    <Panel
      label="磁盘"
      index={10}
      className="h-full"
      hint={
        <>
          <span>{disks.length} 分区</span>
          <span>·</span>
          <span>{disks[0]?.disk_type || '未知'}</span>
        </>
      }
      bodyClassName="gap-3"
    >
      <div className="grid shrink-0 grid-cols-2 gap-2.5">
        <StatCell
          label="总占用"
          value={summary.pct.toFixed(0)}
          unit="%"
          color={percentColor(summary.pct)}
          sub={`${fmtSize(summary.used)} / ${fmtSize(summary.total)}`}
        />
        <StatCell
          label="可用空间"
          value={fmtSize(summary.avail)}
          color="success"
          sub={`总读取 ${fmtSectors(summary.readTotal)}`}
        />
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto pr-1">
        {disks.map((d) => (
          <div key={d.device} className="dm-row flex flex-col gap-2 pb-3">
            <div className="flex items-center gap-2">
              <span className="font-mono text-xs font-medium xl:text-sm">{d.mount}</span>
              <Chip size="sm" variant="secondary" className="text-[9px]">
                {d.fstype}
              </Chip>
              <span className="ml-auto truncate font-mono text-[9px] opacity-25">{d.device}</span>
            </div>

            <MeterRow
              label="空间"
              title={`${fmtSize(d.used_mb)} / ${fmtSize(d.total_mb)}`}
              ratio={d.usage_percent}
              color={percentColor(d.usage_percent)}
              value={`${d.usage_percent.toFixed(0)}%`}
            />
            <MeterRow
              label="Inode"
              title={`${fmtInode(d.inode_used)} / ${fmtInode(d.inode_total)}`}
              ratio={d.inode_percent}
              color={percentColor(d.inode_percent)}
              value={`${d.inode_percent.toFixed(0)}%`}
            />

            <div className="flex flex-wrap gap-x-4 gap-y-0.5 font-mono text-[10px] opacity-35 xl:text-[11px]">
              <span>
                可用 <span className="text-success">{fmtSize(d.available_mb)}</span>
              </span>
              <span>读 {fmtSectors(d.read_sectors)}</span>
              <span>写 {fmtSectors(d.write_sectors)}</span>
            </div>
          </div>
        ))}
      </div>
    </Panel>
  );
}
