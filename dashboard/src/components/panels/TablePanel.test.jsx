import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import TablePanel from "./TablePanel";

const columns = ["Component", "Sign", "Integrand", "Jacobian", "Max Weighted Value", "Impact", "Point"];
const point = {
  kind: "sample_point",
  discrete: [2, 3],
  continuous: [0.9981234567890123, 1.2345678901234567e-8, -12.345678901234567],
  sampling_weight: 7.123456789012345,
};
const state = {
  panel_id: "max_weight_points", columns,
  rows: [["real", "+", 1234.5, 0.00012345, 10500, 0.00001, point]],
  payload: { column_formats: Object.fromEntries(columns.slice(2, 6).map((column) => [column, "scientific"])) },
};

describe("Max Weight Points", () => {
  test("uses scientific columns and three significant digits with a full-precision disclosure", () => {
    const { rerender } = render(<TablePanel title="Max Weight Points" state={state} />);
    const cells = within(screen.getAllByRole("row")[1]).getAllByRole("cell");
    expect(cells.slice(2, 6).map((cell) => cell.textContent)).toEqual([
      "1.234500e+3", "1.234500e-4", "1.050000e+4", "1.000000e-5",
    ]);
    expect(cells[6]).toHaveTextContent("d=[2, 3], c=[0.998, 1.23e-8, -12.3], w=7.123457e+0");
    const toggle = within(cells[6]).getByText("Full Precision (f64)");
    const details = toggle.closest("details");
    expect(details).not.toHaveAttribute("open");
    fireEvent.click(toggle);
    expect(details).toHaveAttribute("open");
    point.continuous.forEach((value) => expect(details).toHaveTextContent(value.toExponential(16)));
    expect(details).toHaveTextContent(`w=${point.sampling_weight.toExponential(16)}`);
    // Polling updates should retain the disclosure state.
    rerender(<TablePanel title="Max Weight Points" state={{ ...state, rows: state.rows.map((row) => [...row]) }} />);
    expect(details).toHaveAttribute("open");
    fireEvent.click(toggle);
    expect(details).not.toHaveAttribute("open");
  });

  test("preserves unavailable metadata and distinguishes it from zero", () => {
    render(<TablePanel title="Max Weight Points" state={{ ...state, rows: [
      ["real", "+", "n/a", "n/a", 0, 0, { ...point, continuous: [0, 1], sampling_weight: null }],
    ] }} />);
    const cells = within(screen.getAllByRole("row")[1]).getAllByRole("cell");
    expect(cells.slice(2, 6).map((cell) => cell.textContent)).toEqual(["n/a", "n/a", "0e+0", "0e+0"]);
    expect(cells[6]).toHaveTextContent("c=[0.00, 1.00], w=n/a");
  });
});

describe("Campaign Sub-runs", () => {
  const columns = ["name", "status", "run", "coeff", "real", "real err", "real err (%)"];
  const rows = [
    ["graph-a", "running", 11, 1, -2, 0.25, 12.5],
    ["graph-b", "waiting", 12, 2, 100, 1, 1],
    ["graph-c", "pending", 13, 1, null, null, null],
    ["graph-d", "waiting", 14, 1, 3, 0.6, 20],
  ];
  const state = {
    panel_id: "campaign_children", columns, rows,
    visible_column_indices: [0, 1, 3, 4, 5, 6],
    payload: {
      sortable: true, row_numbers: true,
      row_action: { kind: "select_run", column: "run" },
      column_formats: { real: "scientific", "real err": "scientific", "abs real": "scientific", "abs real err": "scientific" },
      absolute_components: {
        columns: ["name", "status", "run", "coeff", "abs real", "abs real err", "real err (%)"],
        rows: rows.map((row, index) => index === 0 ? [...row.slice(0, 4), 200, 0.1, 0.05] : row),
      },
    },
  };
  const body = () => screen.getAllByRole("row").slice(1).map((row) => within(row).getAllByRole("cell").map((cell) => cell.textContent));

  test("sorts raw numbers descending then ascending, renumbers rows, and selects the right run", () => {
    const onSelectRun = vi.fn();
    const { rerender } = render(<TablePanel title="Campaign Sub-runs" state={state} onSelectRun={onSelectRun} />);
    fireEvent.click(screen.getByRole("button", { name: "real", exact: true }));
    expect(body().map((row) => row[1])).toEqual(["graph-b", "graph-d", "graph-a", "graph-c"]);
    expect(body().map((row) => row[0])).toEqual(["1", "2", "3", "4"]);
    expect(screen.getByRole("columnheader", { name: "real", exact: true })).toHaveAttribute("aria-sort", "descending");
    fireEvent.click(screen.getByText("graph-b"));
    expect(onSelectRun).toHaveBeenCalledWith(12);
    fireEvent.click(screen.getByRole("button", { name: "real", exact: true }));
    expect(body().map((row) => row[1])).toEqual(["graph-a", "graph-d", "graph-b", "graph-c"]);
    expect(body()[0].slice(4)).toEqual(["-2.000000e+0", "2.500000e-1", "12.5"]);
    rerender(<TablePanel title="Campaign Sub-runs" state={{ ...state, rows: rows.map((row) => [...row]) }} onSelectRun={onSelectRun} />);
    expect(body().map((row) => row[1])).toEqual(["graph-a", "graph-d", "graph-b", "graph-c"]);
    // A new column starts descending, including textual columns.
    fireEvent.click(screen.getByRole("button", { name: "name" }));
    expect(body().map((row) => row[1])).toEqual(["graph-d", "graph-c", "graph-b", "graph-a"]);
  });

  test("switches values, errors, and relative errors together and reapplies the selected sort", () => {
    const { rerender } = render(<TablePanel title="Campaign Sub-runs" state={state} />);
    fireEvent.click(screen.getByRole("button", { name: "real", exact: true }));
    fireEvent.click(screen.getByRole("switch", { name: "Absolute components" }));
    expect(body().map((row) => row[1])).toEqual(["graph-a", "graph-b", "graph-d", "graph-c"]);
    expect(body()[0].slice(4)).toEqual(["2.000000e+2", "1.000000e-1", "0.05"]);
    expect(screen.getByRole("columnheader", { name: "abs real", exact: true })).toHaveAttribute("aria-sort", "descending");
    rerender(<TablePanel title="Campaign Sub-runs" state={{ ...state }} />);
    expect(screen.getByRole("switch", { name: "Absolute components" })).toBeChecked();
    fireEvent.click(screen.getByRole("switch", { name: "Absolute components" }));
    expect(body()[0][1]).toBe("graph-b");
  });

  test("sorts infinite relative errors ahead of finite errors and keeps missing values last", () => {
    render(<TablePanel title="Campaign Sub-runs" state={{ ...state, rows: rows.map((row, i) => i === 0 ? [...row.slice(0, 6), "∞"] : row) }} />);
    fireEvent.click(screen.getByRole("button", { name: "real err (%)" }));
    expect(body().map((row) => row[1])).toEqual(["graph-a", "graph-d", "graph-b", "graph-c"]);
    fireEvent.click(screen.getByRole("button", { name: "real err (%)" }));
    expect(body().map((row) => row[1])).toEqual(["graph-b", "graph-d", "graph-a", "graph-c"]);
  });
});
