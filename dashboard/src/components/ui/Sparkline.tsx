import { useId } from "react";

interface SparklineProps {
  values: number[];
  width?: number;
  height?: number;
  color?: string;
  /** Fixed lower bound for the y axis; defaults to the series minimum. */
  min?: number;
  className?: string;
}

/** Tiny SVG line with an area fill. Scales to its container width. */
export function Sparkline({ values, width = 120, height = 26, color = "var(--cyan)", min, className }: SparklineProps) {
  const gradientId = useId();
  if (values.length < 2) {
    return <svg className={className} width="100%" height={height} viewBox={`0 0 ${width} ${height}`} />;
  }
  let lo = min ?? Infinity;
  let hi = -Infinity;
  for (const v of values) {
    if (min === undefined) lo = Math.min(lo, v);
    hi = Math.max(hi, v);
  }
  if (hi - lo < 1e-9) hi = lo + 1;
  const pad = 2;
  const step = width / (values.length - 1);
  const y = (v: number) => pad + (1 - (v - lo) / (hi - lo)) * (height - pad * 2);
  let d = "";
  values.forEach((v, i) => {
    d += `${i ? "L" : "M"}${(i * step).toFixed(1)},${y(v).toFixed(1)}`;
  });
  const area = `${d}L${width},${height}L0,${height}Z`;
  const lastY = y(values[values.length - 1]);

  return (
    <svg
      className={className}
      width="100%"
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      aria-hidden
    >
      <defs>
        <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor={color} stopOpacity="0.28" />
          <stop offset="1" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={area} fill={`url(#${gradientId})`} />
      <path d={d} fill="none" stroke={color} strokeWidth="1.25" vectorEffect="non-scaling-stroke" />
      <circle cx={width} cy={lastY} r="1.8" fill={color} />
    </svg>
  );
}
