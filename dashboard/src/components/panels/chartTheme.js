// Style chart furniture centrally. Keep scientific colour scales and data intact.
// Applying options also preserves the chart instance when the theme changes.
export function withChartTheme(option, palette, navigation = null) {
  const text = palette.text.secondary;
  const background = palette.background.paper;
  const border = palette.divider;
  const map = (value, style) => value == null ? value
    : Array.isArray(value) ? value.map(style) : style(value, 0);
  const axis = (value) => ({
    ...value,
    axisLabel: { ...value.axisLabel, color: text },
    nameTextStyle: { ...value.nameTextStyle, color: text },
    axisLine: { ...value.axisLine, lineStyle: { ...value.axisLine?.lineStyle, color: text } },
    axisTick: { ...value.axisTick, lineStyle: { ...value.axisTick?.lineStyle, color: text } },
    splitLine: { ...value.splitLine, lineStyle: { ...value.splitLine?.lineStyle, color: border } },
    splitArea: { ...value.splitArea, areaStyle: { ...value.splitArea?.areaStyle, color: ["transparent", palette.action.hover] } },
  });
  return {
    ...option,
    darkMode: palette.mode === "dark",
    backgroundColor: background,
    textStyle: { ...option.textStyle, color: text },
    title: map(option.title, (value) => ({ ...value,
      textStyle: { ...value.textStyle, color: palette.text.primary },
      subtextStyle: { ...value.subtextStyle, color: text },
    })),
    xAxis: map(option.xAxis, axis),
    yAxis: map(option.yAxis, axis),
    legend: map(option.legend, (value, index) => ({ ...value,
      ...(navigation?.legend?.[index] && { selected: navigation.legend[index].selected }),
      textStyle: { ...value.textStyle, color: text },
      inactiveColor: palette.text.disabled, pageTextStyle: { color: text },
      pageIconColor: text, pageIconInactiveColor: palette.text.disabled,
    })),
    tooltip: map(option.tooltip, (value) => ({ ...value,
      backgroundColor: background, borderColor: border,
      textStyle: { ...value.textStyle, color: palette.text.primary },
    })),
    visualMap: map(option.visualMap, (value) => ({ ...value, textStyle: { ...value.textStyle, color: text } })),
    dataZoom: map(option.dataZoom, (value, index) => ({ ...value,
      ...(navigation?.dataZoom?.[index] && Object.fromEntries(
        ["start", "end", "startValue", "endValue", "rangeMode"].map(key => [key, navigation.dataZoom[index][key]])
      )),
      backgroundColor: background, borderColor: border,
      textStyle: { ...value.textStyle, color: text },
      fillerColor: palette.action.selected,
      handleStyle: { ...value.handleStyle, color: background, borderColor: text },
      moveHandleStyle: { ...value.moveHandleStyle, color: text },
      dataBackground: { lineStyle: { color: text }, areaStyle: { color: border } },
      selectedDataBackground: { lineStyle: { color: palette.primary.main }, areaStyle: { color: palette.primary.main } },
    })),
    toolbox: map(option.toolbox, (value) => ({ ...value, iconStyle: { ...value.iconStyle, borderColor: text } })),
  };
}
