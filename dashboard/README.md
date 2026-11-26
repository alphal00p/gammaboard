# GammaBoard Dashboard

React/Vite frontend for the GammaBoard dashboard.

## Development

Start the backend from the repository root:

```bash
./gb start --api-only
```

Then run from `dashboard/`:

```bash
npm ci
npm start
```

Vite proxies `/api` to `http://127.0.0.1:4000`. If its printed browser URL uses a
different port, the default loopback-origin policy already allows it. With
`server.allow_loopback_origins = false`, add that exact origin to the deployment
file and run `./gb config reload --origins` from the repository root. Production assets are
served by GammaBoard; see [deployment](../docs/deployment.md).

## Scripts

```bash
npm start       # Development server
npm run build   # Production build
npm test        # Run tests
```

For dashboard use and measurement definitions, see [operations](../docs/operations.md#read-performance).

The header's sun/moon theme button persists the choice per browser and applies it to the dashboard,
plot labels/backgrounds and TOML editor. Heatmap scales keep their data colours;
supplied SVG documents retain a white page. Chart image exports use the active theme.

Run, task and worker definitions share a lazy-loaded CodeMirror editor with TOML
highlighting, search and undo. Tab moves focus out of the editor. Scope folding is
disabled. Templates (including `$(name:default)` strings) are saved verbatim;
replacement expansion and validation remain on the server.

Shared theme defaults, panel headers, control bars and status chips keep the
workspaces visually consistent. Run selectors show a compact name and state;
their dropdown and hover text retain sample counts, allocation and parent details.

Image components are selected beside image progress. Complex-phase images use a
cyclic phase key (radians) and magnitude saturation: white is zero, full colour is
the largest finite, valid magnitude in the full image. Zooming keeps that scale.
Performance time-window changes retain the previous panels while refreshing,
with an updating notice; the activity table explains the wall-time percentages.

UI presentation is shared through `theme.jsx`, `PanelChrome` and `StatusChip`.
`LazyChart` applies chart themes without changing data colours; `Image2dPanel`
owns raster rendering, navigation and colour keys. `PanelCollection` composes the
server's panel descriptors, including the image progress/selector group.
