import { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import PanelCollection from "./PanelCollection";
import ObservableHistogramPanel from "./ObservableHistogramPanel";
import { observableHistogramView } from "./observableHistograms";
import { buildHistogramData, buildHistogramRenderData } from "./histogramUtils";
import { figureCsv } from "./FigureExportActions";

const { chart } = vi.hoisted(() => ({ chart: vi.fn() }));
vi.mock("./LazyChart", () => ({ default: (props) => { chart(props.option); return <div data-testid="chart" />; } }));
beforeEach(() => chart.mockClear());
const pair = (mean, error, basicMean, basicError) => ({ estimate: { mean, error }, basic: { mean: basicMean, error: basicError } });
const state = {
  panel_id: "observable_selected", source_panel_id: "observable", name: "energy", sample_count: 160,
  bins: [
    { start: 0, stop: 1, value: 3, error: .25, basic: { mean: 4, error: 1 } },
    { start: 1, stop: 2, value: -2, error: 0, basic: { mean: -3, error: .6 } },
    { start: 2, stop: 3, value: 1, error: .5, basic: { mean: 2, error: 1 } },
  ],
  strata: [
    { prefix: [0], probability: .2, sample_count: 64, matches: 32, bins: [pair(1,.1,2,.4), pair(-1,.2,-2,.4), pair(0,0,0,0)] },
    { prefix: [1,3], probability: .8, sample_count: 96, matches: 80, bins: [pair(2,.2,2,.6), pair(-1,.3,-1,.4), pair(1,.5,2,1)] },
  ],
};
const lastChart = () => chart.mock.calls.at(-1)[0];
const series = (name) => lastChart().series.find((s) => s.name === name && s.type === "line");
const chooseDisplay = (label) => {
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "Display" }));
  fireEvent.click(screen.getByRole("option", { name: label }));
};

test("histogram selectors default to one combined curve and compare errors without extra panels", () => {
  render(<ObservableHistogramPanel title="Observable" state={state} />);
  expect(screen.getAllByTestId("chart")).toHaveLength(1);
  expect(series("Stratified").data[0][1]).toBe(3);
  expect(series("Basic (unstratified)")).toBeUndefined();
  chooseDisplay("Compare estimates");
  expect(series("Basic (unstratified)").data[0][1]).toBe(4);
  expect(lastChart().series.some((s) => s.name === "Basic (unstratified) error")).toBe(true);
  chooseDisplay("Compare errors");
  expect(series("Stratified error").data[0][1]).toBe(.25);
  expect(series("Basic error").data[0][1]).toBe(1);
  expect(lastChart().yAxis.min).toBe(0);
  expect(lastChart().series.every((s) => s.type !== "custom")).toBe(true);
  expect(screen.queryByRole("button", { name: "Histogram" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Relative error" })).not.toBeInTheDocument();
  expect(screen.getAllByTestId("chart")).toHaveLength(1);
  fireEvent.change(screen.getByRole("combobox", { name: "Stratum" }), { target: { value: "1,3" } });
  fireEvent.click(screen.getByRole("option", { name: "(1,3)" }));
  expect(series("Stratified error").data[0][1]).toBe(.2);
  expect(series("Basic error").data[0][1]).toBe(.6);
  expect(screen.getByText(/96 draws/)).toBeInTheDocument();
  chooseDisplay("Error reduction");
  expect(series("Basic / stratified error").data[0][1]).toBeCloseTo(3);
  expect(series("Basic / stratified error").markLine.data).toEqual([{ yAxis: 1 }]);
});

test("zero and missing errors leave gaps, while negative means and real zero errors are preserved", () => {
  const view = observableHistogramView(state, { display: "error_ratio" });
  expect(view.state.bins.map((b) => b.value)).toEqual([4, null, 2]);
  const rendered = buildHistogramRenderData(buildHistogramData(view.state.bins), "linear");
  expect(rendered).toHaveLength(6);
  expect(rendered.slice(2,4).every((p) => Number.isNaN(p.y))).toBe(true);
  const partial = { ...state, strata: [{ ...state.strata[0], sample_count: 0, bins: state.bins.map(() => pair(null,null,0,0)) }] };
  const selected = observableHistogramView(partial, { stratum: "0", display: "errors" });
  expect(selected.state.bins.every((b) => b.value === null && b.error === null)).toBe(true);
  render(<ObservableHistogramPanel title="Partial" state={partial} value={{ histogram_view: { observable: { stratum: "0", display: "errors" } } }} onValueChange={() => {}} />);
  expect(screen.getByRole("combobox", { name: "Stratum" })).toBeInTheDocument();
  expect(series("Basic error").data[0][1]).toBe(0);
});

test("selectors are persisted with other histogram preferences and survive switching observables", () => {
  const changes = vi.fn();
  function Harness() {
    const [value, setValue] = useState({ histogram_view: { zoom: [10,80], display_mode: "cdf" } });
    return <ObservableHistogramPanel title="Persistent" state={state} value={value} onValueChange={(id,next) => { changes(id,next); setValue(next); }} />;
  }
  render(<Harness />);
  chooseDisplay("Compare errors");
  expect(changes.mock.calls.at(-1)[0]).toBe("observable");
  expect(changes.mock.calls.at(-1)[1].histogram_view.zoom).toEqual([10,80]);
  chooseDisplay("Estimate");
  expect(screen.getByRole("button", { name: "Cumulative" })).toHaveAttribute("aria-pressed", "true");
  expect(series("Stratified").data.at(-1)[1]).toBe(2);
  expect(observableHistogramView(state, { stratum: "9,9", display: "removed" }).selection).toEqual({stratum:"combined",display:"estimate"});
});

test("exports describe the selected population and estimator rather than the combined view", () => {
  const view = observableHistogramView(state, { stratum:"1,3", display:"errors" });
  const csv = figureCsv({state:view.state});
  expect(csv).toContain('"[1,3]",errors,0.2,,2,0.2,2,0.6');
  expect(view.state.selection).toEqual({stratum:"1,3",display:"errors"});
  expect(view.state.sample_count).toBe(96);
});

test("the table expansion forwards stratum metadata and ordinary histograms remain unchanged", () => {
  const histogram = { sample_count:160, bins:state.bins.map((b) => ({x_min:b.start,x_max:b.stop,sum_weights:160*b.basic.mean,sum_weights_squared:1,estimate:{mean:b.value,error:b.error},basic:b.basic})), strata:state.strata.map((s) => ({bins:s.bins})) };
  const { rerender } = render(<PanelCollection panelSpecs={[{panel_id:"observable",label:"Observables",kind:"table"}]} panelStates={[{panel_id:"observable",kind:"table",columns:["Name"],rows:[["energy"]],payload:{expands_to:{kind:"histogram",source:"selected_row"},strata:state.strata.map((s)=>({prefix:s.prefix,probability:s.probability,sample_count:s.sample_count,matches:s.matches})),histograms:{energy:histogram}}}]} />);
  expect(screen.getByRole("combobox", {name:"Stratum"})).toBeInTheDocument();
  expect(series("Stratified").data[0][1]).toBe(3);
  rerender(<ObservableHistogramPanel title="Ordinary" state={{panel_id:"plain",bins:state.bins}} />);
  expect(screen.queryByRole("combobox", {name:"Stratum"})).not.toBeInTheDocument();
});

test("discrete histograms preserve bin identity and replace obsolete estimate series", () => {
  const discrete = { ...state, bins: state.bins.map((bin,index) => ({...bin,bin_id:index,label:`bin ${index}`})) };
  render(<ObservableHistogramPanel title="Discrete" state={discrete} />);
  chooseDisplay("Compare errors");
  const bars = lastChart().series.filter((s) => s.type === "bar");
  expect(bars.map((s) => s.name)).toEqual(["Stratified error","Basic error"]);
  expect(lastChart().yAxis.min).toBe(0);
  expect(bars[0].data.map((p) => p.value)).toEqual([.25,0,.5]);
  expect(lastChart().xAxis.data).toEqual(["bin 0","bin 1","bin 2"]);
  chooseDisplay("Error reduction");
  expect(lastChart().series).toHaveLength(1);
  expect(lastChart().series[0].data).toEqual([{value:4,rawValue:4},null,{value:2,rawValue:2}]);
});

test("uploaded combined references are retained without comparing incompatible populations or error units", () => {
  render(<ObservableHistogramPanel title="References" state={state} uploadedBundles={[{id:"reference",label:"Independent run",histograms:{energy:{bins:state.bins}}}]} />);
  expect(series("Independent run: energy")).toBeDefined();
  chooseDisplay("Compare errors");
  expect(series("Independent run: energy")).toBeUndefined();
  chooseDisplay("Estimate");
  expect(series("Independent run: energy")).toBeDefined();
  fireEvent.change(screen.getByRole("combobox", {name:"Stratum"}), {target:{value:"1,3"}});
  fireEvent.click(screen.getByRole("option", {name:"(1,3)"}));
  expect(series("Independent run: energy")).toBeUndefined();
  expect(screen.getByText(/Uploaded reference curves are shown/)).toBeInTheDocument();
});

test("bin merging also groups the basic-estimator comparison while error views retain original bins", () => {
  render(<ObservableHistogramPanel title="Observable" state={state} />);
  fireEvent.change(screen.getByRole("spinbutton", { name: "Merge adjacent bins" }), { target: { value: "2" } });
  expect(series("Stratified").data).toEqual([[0, 1], [2, 1], [2, 1], [3, 1]]);
  chooseDisplay("Compare estimates");
  expect(screen.getByRole("spinbutton", { name: "Merge adjacent bins" })).toHaveValue(2);
  expect(series("Basic (unstratified)").data).toEqual([[0, 1], [2, 1], [2, 2], [3, 2]]);
  chooseDisplay("Compare errors");
  expect(screen.queryByRole("spinbutton", { name: "Merge adjacent bins" })).not.toBeInTheDocument();
  expect(series("Stratified error").data[0][1]).toBe(.25);
  chooseDisplay("Estimate");
  expect(screen.getByRole("spinbutton", { name: "Merge adjacent bins" })).toHaveValue(2);
});
