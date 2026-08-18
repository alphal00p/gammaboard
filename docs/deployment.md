# Deployment

`gammaboard deploy` is the normal dashboard stack supervisor. It starts the
backend API, nginx/frontend exposure, local or managed Postgres, and optional
local node runners according to the selected deploy/runtime/server configs.

## Profiles

- `local`: development profile for running from the checkout.
- `itphlies`: foreground-supervised deployment on the ITPhlies server.
- `ubelix`: Slurm-supervised control and worker jobs on UBELIX.

Profile-specific commands stay in `ops/*/README.md`. Shared config semantics are
documented in [config.md](config.md).

## Runtime Layout

Paths are resolved from the configured resources root unless explicitly
absolute. The default resource layout is:

```text
resources/
  runtimes/          Process evaluator/sampler runtime projects and images
  states/            Shared model/integrator/checkpoint state
  templates/         Run, task, and node launch TOML templates
```

Deployment artifacts are separate from user resources:

```text
artifacts/           Build inputs, compiled binaries, package caches
db/                  Local Postgres state
images/              Deployment/service images such as gammaboard.sif and gammaloop.sif
logs/                Postgres, Slurm, and deployment logs
runtime/             Runtime sockets/pids/transient control files
```

## Image Model

There are two different image/artifact roles:

- Service images run GammaBoard infrastructure. `gammaboard.sif` contains the
  Rust backend, nginx/Postgres dependencies, embedded frontend build, and enough
  tooling to launch process workers. `gammaloop.sif` is the GammaLoop execution
  service image.
- Process runtime images run task code. They are task-specific evaluator or
  sampler environments, for example a Python MADNIS sampler image under
  `resources/runtimes/...`.

The service image should be updated when GammaBoard itself changes. A process
runtime image should be updated when the evaluator/sampler implementation or
its dependencies change. The process protocol is the boundary between them, so
multiple runtime images can be used with the same GammaBoard service image.

Current service image builds overwrite the latest image and write a small
`.meta` file. Generic process runtime image builds simply overwrite the
requested runtime `.sif`. Older image generations are intentionally not
retained.

## Symbolica OEM License

GammaBoard and GammaLoop use their bundled Symbolica OEM activation by default.
Users do not need to set a Symbolica environment variable or provide their own
license:

- The binaries call `symbolica::activate_oem_license!(...)` at startup.
- Runtime Slurm jobs do not need `SYMBOLICA_LICENSE`.

To opt out of OEM activation for a local/custom build, set
`NO_SYMBOLICA_OEM_LICENSE=1` while compiling and provide a regular license with
`SYMBOLICA_LICENSE` at runtime. `NO_SYMBOLICA_OEM_LICENSE` is a compile-time
switch; setting it only when launching an already-built binary has no effect.

## Nested Runtimes

Process tasks run explicit commands from their config. A command may directly
start a local binary or script, execute an Apptainer runtime image, or use any
other packaging tool that can start a protocol-speaking process.
For nested Apptainer runtimes, the outer worker image launches the inner runtime
with normal bind paths. GPU-capable workers add NVIDIA passthrough with `--nv`.

The process protocol itself is packaging-agnostic. See
[process-runtime.md](process-runtime.md).

## Nix

The repository flake is a development/operator shell for local work. It provides
compilers and service tools such as Rust, Node.js, PostgreSQL, nginx, Apptainer,
and `sqlx`.

It is not the deployment mechanism. Deployments should be described in terms of
the `gammaboard` binary, runtime/server/deploy config files, and, for UBELIX,
Slurm plus Apptainer service/runtime images.

## Ports

Default ports are:

- Frontend: `8080`
- API: `4000`
- Postgres: `5400`

Deploy helpers support a port offset. Offset `1` shifts these to
`8081/4001/5401` and also suffixes local Postgres state paths where relevant.

## Access

Local and ITPhlies deployments usually expose the frontend directly on the
selected host and port. UBELIX deployments print an SSH tunnel command; open the
frontend through that tunnel from the workstation.
