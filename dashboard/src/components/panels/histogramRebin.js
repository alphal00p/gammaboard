export const sameEdge = (a, b) => Math.abs(a - b) <= 1e-9 * Math.max(Number.MIN_VALUE, Math.abs(a), Math.abs(b));

export const mergeBinGroup = (group) => {
  const value = group.reduce((sum, bin) => sum + bin.value, 0);
  const error = group.every((bin) => bin.error != null && Number.isFinite(bin.error))
    ? group.reduce((total, bin) => Math.hypot(total, bin.error), 0) : null;
  if (!Number.isFinite(value) || (error != null && !Number.isFinite(error))) {
    throw new Error("Merged bin values exceed the supported numeric range.");
  }
  // Cross-bin moments are unavailable. Never present summed second moments as
  // the raw statistics of the new bin; only its display estimate is meaningful.
  const start = group[0].start;
  const stop = group.at(-1).stop;
  return { start, stop, x: start / 2 + stop / 2, value, error };
};

export const rebinHistogramBins = (input, factor = 1, density = false) => {
  if (!Number.isSafeInteger(factor) || factor < 1) throw new Error("Merge factor must be a positive integer.");
  const bins = input.map((bin) => ({ ...bin })).sort((a, b) => a.start - b.start);
  if (factor > 1 && bins.some((bin) => bin.bin_id != null || bin.label != null)) {
    throw new Error("Adjacent-bin merging is available for continuous histograms only.");
  }
  bins.forEach((bin, index) => {
    if (![bin.start, bin.stop, bin.value].every(Number.isFinite) || bin.start >= bin.stop) {
      throw new Error("Rebinning requires finite values and increasing bin edges.");
    }
    if (index > 0 && bins[index - 1].stop > bin.start && !sameEdge(bins[index - 1].stop, bin.start)) {
      throw new Error("Cannot rebin overlapping bins.");
    }
    if (density) {
      bin.value *= bin.stop - bin.start;
      if (bin.error != null) bin.error *= bin.stop - bin.start;
      if (!Number.isFinite(bin.value) || (bin.error != null && !Number.isFinite(bin.error))) {
        throw new Error("Density conversion exceeds the supported numeric range.");
      }
    }
  });
  if (factor === 1) return bins;
  if (factor > bins.length) throw new Error("Merge factor cannot exceed the number of bins.");
  const result = [];
  for (let first = 0; first < bins.length; first += factor) {
    const group = bins.slice(first, first + factor);
    if (group.some((bin, index) => index > 0 && !sameEdge(group[index - 1].stop, bin.start))) {
      throw new Error("Cannot merge bins across a gap.");
    }
    result.push(mergeBinGroup(group));
  }
  return result;
};

// Match whole intervals or coarsen a finer reference exactly. Never sample a
// midpoint or split an integrated bin to fabricate a comparison at another width.
export const alignComparisonBins = (reference, overlay) => {
  let index = 0;
  return reference.map((bin) => {
    while (index < overlay.length && overlay[index].stop <= bin.start && !sameEdge(overlay[index].start, bin.start)) index += 1;
    let next = index;
    if (!overlay[next] || !sameEdge(overlay[next].start, bin.start)) return null;
    const group = [];
    while (next < overlay.length) {
      const candidate = overlay[next];
      if (group.length && !sameEdge(group.at(-1).stop, candidate.start)) return null;
      if (candidate.stop > bin.stop && !sameEdge(candidate.stop, bin.stop)) return null;
      group.push(candidate);
      if (sameEdge(candidate.stop, bin.stop)) {
        try { return mergeBinGroup(group); } catch { return null; }
      }
      next += 1;
    }
    return null;
  });
};

export const alignCumulativeBins = (reference, overlay) => {
  if (!reference.length || !overlay.length || !sameEdge(reference[0].start, overlay[0].start)) {
    return reference.map(() => null);
  }
  let index = 0;
  return reference.map((bin) => {
    while (index < overlay.length && overlay[index].stop < bin.stop && !sameEdge(overlay[index].stop, bin.stop)) index += 1;
    return overlay[index] && sameEdge(overlay[index].stop, bin.stop) ? overlay[index] : null;
  });
};
