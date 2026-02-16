/**
 * Parses raw sample data from the API into a format suitable for charting
 * @param {Array} samplesData - Raw samples from the API
 * @returns {Array} Parsed samples with x, value, weight, and index
 */
export const parseSamples = (samplesData) => {
  return samplesData.map((sample, idx) => {
    // Extract x value from point (handle different formats)
    let x = idx;
    if (typeof sample.point === "number") {
      x = sample.point;
    } else if (Array.isArray(sample.point)) {
      x = sample.point[0];
    } else if (sample.point && typeof sample.point === "object" && sample.point.x !== undefined) {
      x = sample.point.x;
    }

    return {
      x: x,
      value: sample.value,
      weight: sample.weight,
      index: idx,
    };
  });
};
