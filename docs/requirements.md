# Supported Environment

GammaBoard's supported local workflow is Linux on x86_64 using the repository
Nix development shell. Nix is recommended because it provides the exact Rust,
Node.js, PostgreSQL, nginx, compiler, and scientific build tools used by the
checked-in workflows.

Without Nix, install the following before using `gammaboard deploy`:

- Current stable Rust with Cargo and a C/C++ toolchain.
- Python 3.11 or newer for `gammaboard-process`; the MADNIS integration supports
  Python 3.12 and 3.13.
- Node.js at the version in `dashboard/.nvmrc` and npm.
- PostgreSQL 16 client and server tools, including `initdb`, `pg_ctl`,
  `createdb`, and `psql`.
- `sqlx-cli` with PostgreSQL support and nginx.

The installation smoke test needs a few hundred MB for Rust build artifacts,
the dashboard dependencies, and a local PostgreSQL cluster. Physics examples
can require substantially more disk space for Apptainer images, LHAPDF data,
generated source, and integration state.

## Reproducibility Metadata

Each created run stores its submitted TOML, normalized effective TOML,
GammaBoard version, compile-time Git revision, enabled Cargo features, and the
JSON object supplied through `GAMMABOARD_EXTERNAL_VERSIONS`. Set that variable
before creating a run when external tools are involved, for example:

```bash
export GAMMABOARD_EXTERNAL_VERSIONS='{
  "gammaloop": "<commit-or-release>",
  "madgraph": "<commit-or-release>",
  "lhapdf_set": "NNPDF31_nnlo_as_0118"
}'
```

The dashboard API exposes the stored provenance through each run response and
includes it in the run reproduction TOML export.
