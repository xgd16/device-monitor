import { useState, useEffect } from 'react';
import { Spinner } from '@heroui/react';
import { TrendMetricCard } from './TrendMetricCard';
import type { CpuInfo, CpuGovernor } from '../types';

interface CpuCardProps {
  cpu: CpuInfo;
  history: number[];
  timestamps?: number[];
  loadAvg: number[];
}

const governorDescriptions: Record<string, string> = {
  performance: '最高性能',
  powersave: '省电',
  schedutil: '智能调度',
  ondemand: '按需调频',
  conservative: '保守调频',
  userspace: '用户控制',
};

export function CpuCard({ cpu, history, timestamps, loadAvg }: CpuCardProps) {
  const [governor, setGovernor] = useState<CpuGovernor | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<'normal' | 'low-power'>('normal');

  useEffect(() => {
    fetchGovernor();
  }, []);

  const fetchGovernor = async () => {
    try {
      const res = await fetch('/api/cpu/governor');
      const data = await res.json();
      if (data.code === 0) {
        setGovernor(data.data);
        setMode(data.data.current === 'userspace' ? 'low-power' : 'normal');
      }
    } catch (err) {
      console.error('Failed to fetch governor:', err);
    }
  };

  const handleGovernorChange = async (newGovernor: string) => {
    if (!newGovernor || newGovernor === governor?.current) return;

    setLoading(true);
    setError(null);

    try {
      const res = await fetch('/api/cpu/governor', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ governor: newGovernor }),
      });
      const data = await res.json();

      if (data.code === 0) {
        setGovernor(data.data.governor);
        setMode(newGovernor === 'userspace' ? 'low-power' : 'normal');
      } else {
        setError(data.error || '设置失败');
        setTimeout(() => setError(null), 3000);
      }
    } catch {
      setError('网络错误');
      setTimeout(() => setError(null), 3000);
    } finally {
      setLoading(false);
    }
  };

  const handleLowPowerMode = async () => {
    setLoading(true);
    setError(null);

    try {
      const res = await fetch('/api/cpu/low-power', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      });
      const data = await res.json();

      if (data.code === 0) {
        setGovernor(data.data.governor);
        setMode('low-power');
      } else {
        setError(data.error || '设置失败');
        setTimeout(() => setError(null), 3000);
      }
    } catch {
      setError('网络错误');
      setTimeout(() => setError(null), 3000);
    } finally {
      setLoading(false);
    }
  };

  const handleNormalMode = async () => {
    setLoading(true);
    setError(null);

    try {
      const res = await fetch('/api/cpu/normal', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
      });
      const data = await res.json();

      if (data.code === 0) {
        setGovernor(data.data.governor);
        setMode('normal');
      } else {
        setError(data.error || '设置失败');
        setTimeout(() => setError(null), 3000);
      }
    } catch {
      setError('网络错误');
      setTimeout(() => setError(null), 3000);
    } finally {
      setLoading(false);
    }
  };

  const headerExtra = (
    <>
      {loading && <Spinner size="sm" />}
      <div className="inline-flex rounded-md border border-default-200 overflow-hidden text-[10px] font-mono shrink-0">
        <button
          type="button"
          onClick={handleLowPowerMode}
          disabled={loading || mode === 'low-power'}
          className={`px-2 py-0.5 transition-colors disabled:opacity-50 ${
            mode === 'low-power' ? 'bg-warning text-warning-foreground' : 'hover:bg-content2'
          }`}
        >
          省电
        </button>
        <button
          type="button"
          onClick={handleNormalMode}
          disabled={loading || mode === 'normal'}
          className={`px-2 py-0.5 border-l border-default-200 transition-colors disabled:opacity-50 ${
            mode === 'normal' ? 'bg-success text-success-foreground' : 'hover:bg-content2'
          }`}
        >
          正常
        </button>
      </div>
      {governor && (
        <select
          value={governor.current}
          onChange={(e) => handleGovernorChange(e.target.value)}
          disabled={loading}
          className="max-w-36 sm:max-w-44 px-1.5 py-0.5 text-[10px] font-mono bg-content2 border border-default-200 rounded-md focus:outline-none focus:ring-1 focus:ring-primary disabled:opacity-50 truncate"
        >
          {governor.available.map((gov) => (
            <option key={gov} value={gov}>
              {gov}
              {governorDescriptions[gov] ? ` · ${governorDescriptions[gov]}` : ''}
            </option>
          ))}
        </select>
      )}
    </>
  );

  return (
    <TrendMetricCard
      title="处理器"
      value={cpu.overall_usage}
      variant="cpu"
      history={history}
      timestamps={timestamps}
      headerExtra={headerExtra}
      banner={
        error ? (
          <div className="text-[10px] text-danger bg-danger-50 px-2 py-1 rounded -mt-1">{error}</div>
        ) : undefined
      }
      footer={
        <>
          <span>负载 {loadAvg.map((v) => v.toFixed(2)).join(' / ')}</span>
          <span className="mx-2 opacity-30">·</span>
          <span>{cpu.cores[0]?.frequency_mhz || 0} MHz</span>
          <span className="mx-2 opacity-30">·</span>
          <span>{cpu.cores.length} 核</span>
        </>
      }
    />
  );
}
