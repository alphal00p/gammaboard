import { Autocomplete, Button, MenuItem, Stack, TextField, Typography } from '@mui/material';
import { prefixLabel } from './strataUtils';
import {
  FULL_ZOOM,
  buildDataZoom,
  normalizeZoomRange,
  readDataZoomRanges,
  zoomRangeChanged
} from './panelView';
export const STRATA_PAGE_SIZE = 4;
const comparisonPage = (page, count) =>
  Math.max(0, Math.min(Math.floor(Number(page) || 0), Math.ceil(count / STRATA_PAGE_SIZE) - 1));
export const scopeIndices = (scope, count, page = 0) => {
  if (scope === 'overall' || !count) return [];
  if (scope === 'compare') {
    const start = comparisonPage(page, count) * STRATA_PAGE_SIZE;
    return Array.from({ length: Math.min(STRATA_PAGE_SIZE, count - start) }, (_, i) => start + i);
  }
  const i = Number(scope);
  return Number.isInteger(i) && i >= 0 && i < count ? [i] : [0];
};
export function DiagnosticSelect({ label, options, value, onChange, minWidth = 180 }) {
  if (options.length > 20)
    return (
      <Autocomplete
        size="small"
        disableClearable
        sx={{
          minWidth
        }}
        options={options}
        value={options.find((o) => o.value === value) || options[0]}
        isOptionEqualToValue={(a, b) => a.value === b.value}
        getOptionLabel={(o) => o.label}
        onChange={(_event, o) => onChange(o.value)}
        renderInput={(params) => <TextField {...params} label={label} />}
      />
    );
  return (
    <TextField
      select
      label={label}
      size="small"
      sx={{
        minWidth
      }}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    >
      {options.map((o) => (
        <MenuItem key={o.value} value={o.value}>
          {o.label}
        </MenuItem>
      ))}
    </TextField>
  );
}
export function ScopeSelector({ prefixes, scope, page = 0, onChange, overall = true }) {
  if (!prefixes.length) return null;
  const groups = Math.ceil(prefixes.length / STRATA_PAGE_SIZE);
  const index = comparisonPage(page, prefixes.length);
  return (
    <Stack direction="row" gap={1} alignItems="center" flexWrap="wrap">
      <DiagnosticSelect
        label="Scope"
        value={scope}
        onChange={(next) =>
          onChange({
            scope: next
          })
        }
        options={[
          ...(overall
            ? [
                {
                  value: 'overall',
                  label: 'Overall'
                }
              ]
            : []),
          {
            value: 'compare',
            label: 'Compare strata'
          },
          ...prefixes.map((p, k) => ({
            value: String(k),
            label: `Stratum ${prefixLabel(p)}`
          }))
        ]}
      />
      {scope === 'compare' && groups > 1 && (
        <>
          <Button
            disabled={!index}
            onClick={() =>
              onChange({
                page: index - 1
              })
            }
          >
            Previous
          </Button>
          <Typography variant="body2">
            {index * STRATA_PAGE_SIZE + 1}–
            {Math.min(prefixes.length, (index + 1) * STRATA_PAGE_SIZE)} of {prefixes.length}
          </Typography>
          <Button
            disabled={index + 1 === groups}
            onClick={() =>
              onChange({
                page: index + 1
              })
            }
          >
            Next
          </Button>
        </>
      )}
    </Stack>
  );
}
export const navigationZoom = (value) =>
  buildDataZoom(value?.zoom ?? FULL_ZOOM, true, true, value?.yZoom ?? FULL_ZOOM, true).map((z) =>
    z.type === 'inside'
      ? {
          ...z,
          zoomOnMouseWheel: 'ctrl',
          moveOnMouseMove: true
        }
      : z
  );
export const navigationEvents = (value, update) => ({
  datazoom: (event) => {
    const ranges = readDataZoomRanges(event),
      patch = {};
    if (ranges.x && zoomRangeChanged(ranges.x, normalizeZoomRange(value?.zoom) || FULL_ZOOM))
      patch.zoom = ranges.x;
    if (ranges.y && zoomRangeChanged(ranges.y, normalizeZoomRange(value?.yZoom) || FULL_ZOOM))
      patch.yZoom = ranges.y;
    if (Object.keys(patch).length) update(patch);
  }
});
