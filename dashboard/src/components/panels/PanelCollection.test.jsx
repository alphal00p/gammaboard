import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import PanelCollection from "./PanelCollection";

vi.mock("./LazyChart", () => ({
  default: () => <div data-testid="histogram-chart" />,
}));

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    disconnect() {}
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const imageSpec = { panel_id: "pdf_adaptation_pdf", label: "Sampler PDF", kind: "image2d" };
const histogramSpec = { panel_id: "pdf_histogram", label: "PDF Histogram", kind: "histogram" };

describe("PanelCollection plot availability", () => {
  test.each([imageSpec, histogramSpec])("keeps $kind panels visible without an update", (spec) => {
    render(<PanelCollection panelSpecs={[spec]} panelStates={[]} />);

    expect(screen.getByText(spec.label)).toBeInTheDocument();
    expect(screen.getByText("No plot data available.")).toBeInTheDocument();
  });

  test.each([
    { spec: imageSpec, state: { width: 128, height: 128, values: [] } },
    { spec: histogramSpec, state: { bins: [] } },
  ])("keeps empty $spec.kind payloads visible", ({ spec, state }) => {
    render(
      <PanelCollection panelSpecs={[spec]} panelStates={[{ ...state, panel_id: spec.panel_id }]} />,
    );

    expect(screen.getByText(spec.label)).toBeInTheDocument();
    expect(screen.getByText("No plot data available.")).toBeInTheDocument();
  });

  test("replaces missing-data messages when image and histogram updates arrive", () => {
    const panelSpecs = [imageSpec, histogramSpec];
    const { container, rerender } = render(<PanelCollection panelSpecs={panelSpecs} panelStates={[]} />);
    expect(screen.getAllByText("No plot data available.")).toHaveLength(2);

    rerender(
      <PanelCollection
        panelSpecs={panelSpecs}
        panelStates={[
          {
            panel_id: imageSpec.panel_id,
            width: 2,
            height: 2,
            values: [9.82e-18, 1e-12, 1e-11, 7.19e-10],
            x_range: [0, 1],
            y_range: [0, 1],
          },
          {
            panel_id: histogramSpec.panel_id,
            controls: { default_relative_error: false },
            bins: [
              { start: 0, stop: 0.5, value: 0.25 },
              { start: 0.5, stop: 1, value: 0.75 },
            ],
          },
        ]}
      />,
    );

    expect(screen.queryByText("No plot data available.")).not.toBeInTheDocument();
    expect(screen.getByText(imageSpec.label)).toBeInTheDocument();
    expect(container.querySelector("canvas")).toBeInTheDocument();
    expect(screen.getByText(histogramSpec.label)).toBeInTheDocument();
    expect(screen.getAllByTestId("histogram-chart")).toHaveLength(1);
  });
});
