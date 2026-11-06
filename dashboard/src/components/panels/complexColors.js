const hsvToRgb = (hue, saturation, value) => {
  const h = ((hue % 1) + 1) % 1;
  const s = Math.max(0, Math.min(1, saturation));
  const v = Math.max(0, Math.min(1, value));
  const sector = h * 6;
  const i = Math.floor(sector);
  const f = sector - i;
  const p = v * (1 - s);
  const q = v * (1 - f * s);
  const t = v * (1 - (1 - f) * s);
  const [r, g, b] =
    i % 6 === 0
      ? [v, t, p]
      : i % 6 === 1
        ? [q, v, p]
        : i % 6 === 2
          ? [p, v, t]
          : i % 6 === 3
            ? [p, q, v]
            : i % 6 === 4
              ? [t, p, v]
              : [v, p, q];
  return [Math.round(r * 255), Math.round(g * 255), Math.round(b * 255)];
};

export const complexPhaseColorForValue = (re, im, maxMagnitude) => {
  const real = Number(re);
  const imag = Number(im);
  if (!Number.isFinite(real) || !Number.isFinite(im)) return [255, 0, 255];
  const magnitude = Math.hypot(real, imag);
  const hue = (Math.atan2(imag, real) + Math.PI) / (2 * Math.PI);
  const saturation = maxMagnitude > 0 ? Math.min(1, magnitude / maxMagnitude) : 0;
  return hsvToRgb(hue, saturation, 1);
};
