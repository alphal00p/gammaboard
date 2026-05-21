import { render, screen, waitFor } from "@testing-library/react";
import { forwardRef } from "react";
import { describe, expect, test, vi } from "vitest";
import HistogramPanel from "./HistogramPanel";

vi.mock("echarts-for-react", () => ({
  default: forwardRef(function MockECharts(_props, ref) {
    return <div ref={ref} data-testid="echarts-mock" />;
  }),
}));

vi.mock("../../lib/echarts", () => ({}));

const baseContinuousBins = [
  { start: 0, stop: 0.5, x: 0.25, value: 1.0, error: 0.1 },
  { start: 0.5, stop: 1.0, x: 0.75, value: 2.0, error: 0.2 },
];

const baseDiscreteBins = [
  {
    bin_id: "[0]",
    label: "d0=0",
    start: 0,
    stop: 1,
    value: 0.25,
    error: 0.02,
    pdf_scaled: 0.3,
    pdf_delta: -0.05,
    entry_count: 12,
  },
  {
    bin_id: "[1]",
    label: "d0=1",
    start: 1,
    stop: 2,
    value: 0.75,
    error: 0.03,
    pdf_scaled: 0.7,
    pdf_delta: 0.05,
    entry_count: 18,
  },
];

const renderHistogram = async (state) => {
  render(<HistogramPanel title="Histogram smoke test" state={state} />);
  expect(screen.getByText(/Histogram smoke test/)).toBeInTheDocument();
  await waitFor(() => {
    expect(screen.getAllByTestId("echarts-mock").length).toBeGreaterThan(0);
  });
};

describe("HistogramPanel smoke tests", () => {
  test("renders continuous histogram with shared-edge overlay", async () => {
    await renderHistogram({
      panel_id: "continuous_histogram",
      name: "continuous",
      bins: baseContinuousBins,
      overlay_alignment: "shared_edges",
      overlay_histograms: [
        {
          name: "comparison",
          bins: baseContinuousBins.map((bin) => ({
            ...bin,
            value: bin.value * 1.1,
            error: bin.error * 1.2,
          })),
        },
      ],
    });
  });

  test("renders discrete pdf-comparison histogram", async () => {
    await renderHistogram({
      panel_id: "discrete_pdf_histogram",
      name: "discrete",
      discrete_ordering: true,
      bins: baseDiscreteBins,
      controls: {
        scale: true,
        x_scale: true,
        pdf_cdf: true,
        sort: true,
        relative_error: true,
        ratio: true,
        export: false,
        reset_view: false,
      },
      metric_descriptors: {
        value: { label: "Value", short_label: "value" },
        pdf_scaled: { label: "Scaled PDF", short_label: "pdf" },
        pdf_delta: { label: "Value - PDF", short_label: "delta" },
      },
      views: [
        {
          id: "pdf_compare",
          label: "PDF Compare",
          kind: "bar_with_marker",
          default: true,
          value_metric: "value",
          error_metric: "error",
          marker_metric: "pdf_scaled",
          delta_metric: "pdf_delta",
          tooltip_metrics: ["value", "pdf_scaled", "pdf_delta"],
        },
      ],
    });
  });
});
