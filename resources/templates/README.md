# Dashboard catalog

`runs/` is the single source for bundled dashboard run templates. The server lists
top-level TOML files there; `runs/support/` contains referenced child cards and
runtime assets and is not shown in the menu. See the
[catalog and prerequisites](../../docs/quickstart.md#default-examples).

`tasks/` contains generic snippets to append to an existing run. `nodes/` contains
local worker-launch presets; deployment profiles may supply their own node cards.

A template is a recipe, not an existing run. Creating a run records its effective
definition in the database. Editing the template does not change existing runs,
and starting a server does not automatically create the showcase.

The local server reads these files through its resource root. UBELIX installs a
portable subset of this same run catalog and its own worker presets into the
workspace. Workspace copies are editable and are not refreshed by a Git pull;
use `install-examples` or reseed the template directory as described in the
[UBELIX guide](../../ops/slurm/README.md#example-runs).
