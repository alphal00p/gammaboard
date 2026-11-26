import * as echarts from "echarts/core";
import { BarChart, CustomChart, HeatmapChart, LineChart, ScatterChart } from "echarts/charts";
import {
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  MarkAreaComponent,
  MarkLineComponent,
  ToolboxComponent,
  TooltipComponent,
  VisualMapComponent,
} from "echarts/components";
import { CanvasRenderer, SVGRenderer } from "echarts/renderers";

echarts.use([
  LineChart,
  BarChart,
  CustomChart,
  HeatmapChart,
  ScatterChart,
  GridComponent,
  LegendComponent,
  MarkAreaComponent,
  MarkLineComponent,
  ToolboxComponent,
  TooltipComponent,
  DataZoomComponent,
  VisualMapComponent,
  CanvasRenderer,
  SVGRenderer,
]);

export { echarts };
