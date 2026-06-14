interface SparklineProps {
  data: number[];
  color: string;
  height?: number;
}

const COLOR_MAP: Record<string, string> = {
  cpu: '#38bdf8',    // sky-400
  mem: '#4ade80',    // green-400
};

export function Sparkline({ data, color, height = 48 }: SparklineProps) {
  if (data.length < 2) return <div style={{ height }} />;
  const mn = Math.min(...data);
  const mx = Math.max(...data);
  const range = mx - mn || 1;
  const w = 320;
  const step = w / (data.length - 1);
  const pts = data.map((v, i) => `${i * step},${height - 2 - ((v - mn) / range) * (height - 4)}`);
  const line = `M${pts.join('L')}`;
  const fill = `M0,${height}L${pts.join('L')}L${w},${height}Z`;
  const resolved = COLOR_MAP[color] || color;
  const id = `sg-${color}`;
  return (
    <svg viewBox={`0 0 ${w} ${height}`} style={{ width: '100%', height, display: 'block' }} preserveAspectRatio="none">
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={resolved} stopOpacity="0.4" />
          <stop offset="100%" stopColor={resolved} stopOpacity="0.02" />
        </linearGradient>
      </defs>
      <path d={fill} fill={`url(#${id})`} />
      <path d={line} fill="none" stroke={resolved} strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}
