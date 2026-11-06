import { createTheme } from "@mui/material/styles";
import { expect, test } from "vitest";
import { withChartTheme } from "./chartTheme";

test("themes chart controls without changing data, colour scales, formatters or navigation", () => {
  const formatter = (value) => `value ${value}`;
  const option = {
    xAxis: { type: "value", axisLabel: { formatter }, splitLine: { show: false } },
    yAxis: [{ type: "log" }, { type: "value", name: "Target" }],
    legend: { selected: { Target: false } },
    tooltip: { formatter },
    dataZoom: [{ type: "slider", start: 20, end: 70 }],
    visualMap: [{ inRange: { color: ["#fff", "#0072b2"] }, min: 0, max: 1 }],
    series: [{ data: [[1, 2]], itemStyle: { color: "#0072b2" } }],
  };
  for (const mode of ["light", "dark"]) {
    const { palette } = createTheme({ palette: { mode } });
    const styled = withChartTheme(option, palette);
    expect(styled.backgroundColor).toBe(palette.background.paper);
    expect(styled.xAxis.axisLabel.color).toBe(palette.text.secondary);
    expect(styled.yAxis[1].nameTextStyle.color).toBe(palette.text.secondary);
    expect(styled.xAxis.splitLine.show).toBe(false);
    expect(styled.tooltip.formatter).toBe(formatter);
    expect(styled.tooltip.textStyle.color).toBe(palette.text.primary);
    expect(styled.legend.selected.Target).toBe(false);
    expect(styled.dataZoom[0]).toMatchObject({ start: 20, end: 70 });
    expect(styled.series).toBe(option.series);
    expect(styled.visualMap[0].inRange).toBe(option.visualMap[0].inRange);
  }
  expect(option.xAxis.axisLabel.color).toBeUndefined();
  const live = { legend: [{ selected: { Target: true } }], dataZoom: [{ start: 35, end: 45 }] };
  const restyled = withChartTheme(option, createTheme().palette, live);
  expect(restyled.legend.selected.Target).toBe(true);
  expect(restyled.dataZoom[0]).toMatchObject({ start: 35, end: 45 });
});
