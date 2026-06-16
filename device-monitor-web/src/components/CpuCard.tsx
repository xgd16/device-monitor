import { useState, useEffect } from 'react';
import { Card, ProgressCircle, Spinner } from '@heroui/react';
import { TrendChart } from './TrendChart';
import { percentColor } from './utils';
import type { CpuInfo, CpuGovernor } from '../types';

interface CpuCardProps {
  cpu: CpuInfo;
  history: number[];
  timestamps?: number[];
  loadAvg: number[];
}

// Governor 描述
const governorDescriptions: Record<string, string> = {
  performance: '最高性能',
  powersave: '省电模式',
  schedutil: '智能调度',
  ondemand: '按需调频',
  conservative: '保守调频',
  userspace: '用户控制',
};

export function CpuCard({ cpu, history, timestamps, loadAvg }: CpuCardProps) {
  const color = percentColor(cpu.overall_usage);
  const [governor, setGovernor] = useState<CpuGovernor | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<'normal' | 'low-power'>('normal');

  // 获取 governor 信息
  useEffect(() => {
    fetchGovernor();
  }, []);

  const fetchGovernor = async () => {
    try {
      const res = await fetch('/api/cpu/governor');
      const data = await res.json();
      if (data.code === 0) {
        setGovernor(data.data);
        // 判断当前模式
        if (data.data.current === 'userspace') {
          setMode('low-power');
        } else {
          setMode('normal');
        }
      }
    } catch (err) {
      console.error('Failed to fetch governor:', err);
    }
  };

  // 设置 governor
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
        if (newGovernor === 'userspace') {
          setMode('low-power');
        } else {
          setMode('normal');
        }
      } else {
        setError(data.error || '设置失败');
        setTimeout(() => setError(null), 3000);
      }
    } catch (err) {
      setError('网络错误');
      setTimeout(() => setError(null), 3000);
    } finally {
      setLoading(false);
    }
  };

  // 切换到低频模式
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
    } catch (err) {
      setError('网络错误');
      setTimeout(() => setError(null), 3000);
    } finally {
      setLoading(false);
    }
  };

  // 恢复正常模式
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
    } catch (err) {
      setError('网络错误');
      setTimeout(() => setError(null), 3000);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">处理器</span>
        {governor && (
          <div className="flex items-center gap-2">
            {loading && <Spinner size="sm" />}
            <select
              value={governor.current}
              onChange={(e) => handleGovernorChange(e.target.value)}
              disabled={loading}
              className="px-2 py-1 text-xs font-mono bg-content2 border border-default-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary disabled:opacity-50"
            >
              {governor.available.map((gov) => (
                <option key={gov} value={gov}>
                  {gov} {governorDescriptions[gov] ? `(${governorDescriptions[gov]})` : ''}
                </option>
              ))}
            </select>
          </div>
        )}
      </div>

      {/* 快捷模式切换按钮 */}
      <div className="flex gap-2">
        <button
          onClick={handleLowPowerMode}
          disabled={loading || mode === 'low-power'}
          className={`flex-1 px-3 py-1.5 text-xs font-mono rounded-lg transition-colors ${
            mode === 'low-power'
              ? 'bg-warning text-warning-foreground'
              : 'bg-content2 hover:bg-content3 disabled:opacity-50'
          }`}
        >
          🔋 低频省电
        </button>
        <button
          onClick={handleNormalMode}
          disabled={loading || mode === 'normal'}
          className={`flex-1 px-3 py-1.5 text-xs font-mono rounded-lg transition-colors ${
            mode === 'normal'
              ? 'bg-success text-success-foreground'
              : 'bg-content2 hover:bg-content3 disabled:opacity-50'
          }`}
        >
          ⚡ 正常模式
        </button>
      </div>

      {error && (
        <div className="text-xs text-danger bg-danger-50 px-2 py-1 rounded">
          {error}
        </div>
      )}

      <div className="flex items-center gap-4 sm:gap-6">
        <div className="relative inline-flex items-center justify-center">
          <ProgressCircle
            value={cpu.overall_usage}
            size="lg"
            color={color as any}
          >
            <ProgressCircle.Track className="size-24 sm:size-28">
              <ProgressCircle.TrackCircle />
              <ProgressCircle.FillCircle />
            </ProgressCircle.Track>
          </ProgressCircle>
          <span className="absolute text-xl sm:text-2xl font-mono font-light">
            {cpu.overall_usage.toFixed(0)}%
          </span>
        </div>
        <div className="flex-1 min-w-0">
          <TrendChart data={history} timestamps={timestamps} variant="cpu" />
          <div className="mt-1.5 flex flex-wrap gap-x-4 gap-y-0.5 text-[10px] sm:text-[11px] font-mono opacity-50">
            <span>负载 {loadAvg.map(v => v.toFixed(2)).join(' / ')}</span>
            <span>{cpu.cores[0]?.frequency_mhz || 0} MHz</span>
          </div>
        </div>
      </div>
    </Card>
  );
}
