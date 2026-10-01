"""Sparse capability curves and directly usable configuration summaries."""

import json
from . import common as bench
from .frontier import adequate, choose, summarize
from .reporting import publish, figure_save


def cost_label(us):
    return f"{us:g} µs" if us < 1000 else f"{us/1000:g} ms"


def report(directory):
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    manifest = json.loads((directory / "manifest.json").read_text())
    records = bench.load_results(directory)
    summary = summarize(records)
    bench.write_json(directory / "summary.json", summary)
    rows = summary["frontier"]
    modes = manifest["suite"]["modes"]
    fig, axes = plt.subplots(1, len(modes), figsize=(6 * len(modes), 5), squeeze=False, sharey=True)
    colors = ["#6d28d9", "#2563eb", "#08916b", "#c2410c"]
    table = []
    selected = []
    for ax, mode in zip(axes[0], modes):
        for index, cost in enumerate(sorted(manifest["suite"]["eval_us"])):
            data = sorted(
                (r for r in rows if r["point"]["mode"] == mode and r["point"]["eval_us"] == cost),
                key=lambda r: r["point"]["workers"],
            )
            if not data:
                continue
            counts = [r["point"]["workers"] for r in data]
            ax.plot(
                counts,
                [r["rate"] / 1e6 for r in data],
                "o-",
                color="black" if cost == 0 else colors[(index - 1) % len(colors)],
                label=cost_label(cost),
            )
            if cost:
                ax.plot(
                    counts,
                    [n / cost for n in counts],
                    ":",
                    color=colors[(index - 1) % len(colors)],
                    alpha=0.45,
                )
            winner = choose(data)
            p = winner["point"]
            selected.append(winner)
            table.append(
                dict(
                    mode=mode,
                    delay_us=cost,
                    evaluators=p["workers"],
                    batch_size=p["batch"],
                    samples_per_second=winner["rate"],
                    batches_per_second=winner["batches_per_second"],
                )
            )
            bench.write_json(
                directory / f"selected-{mode}-{cost:g}us.json",
                dict(point=p, queue=winner["settings"]),
            )
        ax.set_title(
            {
                "materialized": "Without training feedback",
                "training": "With training feedback",
                "rng": "Compact RNG inference",
            }[mode]
        )
        ax.set_xscale("log", base=2)
        ax.set_yscale("log")
        ax.grid(alpha=0.2)
        ax.set_xticks(manifest["workers"], labels=manifest["workers"])
        ax.set_xlabel("Evaluators")
        ax.set_ylabel("Million accepted samples/s")
        if ax.lines:
            ax.legend(title="Delay / sample")
        else:
            ax.text(0.5, 0.5, "Not measured", ha="center", transform=ax.transAxes)
    fig.suptitle("GammaBoard scaling · sparse curves and zero-delay ceiling")
    if manifest.get("plot_note"):
        fig.text(0.5, 0.01, manifest["plot_note"], ha="center", fontsize=9)
    fig.tight_layout(rect=(0, 0.05 if manifest.get("plot_note") else 0, 1, 1))
    figure_save(fig, directory, "frontier")
    plt.close(fig)
    bench.write_json(directory / "selected.json", selected)
    notes = [
        f'{len(records)} recorded points; {sum(r["valid"] and adequate(r) for r in records)} valid and adequately sampled. Unmeasured or failed configurations remain explicit in the manifest and raw records.',
        "Materialized six-dimensional samples; feedback is f(x)=x[0]. Training measures feedback traffic and ingestion, without optimizer barriers. Compact RNG inference is optional.",
        "Delays are batch sleeps with seeded 10% jitter, without CPU arithmetic. Dotted lines show ideal delay-limited rates. The zero-delay curve measures the full pipeline overhead ceiling.",
        "Accepted sample counters and busy time share sampler telemetry endpoints. Warmup drains old generated work; intervals cover at least eight accepted/completed batches and six nominal batch durations. Observations cover two nominal generation cycles; discrepant intervals use complete generation boundaries when available. Accepted/evaluated deltas must agree within the completion-boundary allowance.",
        "These sparse curves reuse selected batch/queue settings; they do not optimize every configuration. Within 5% of the observed peak, the table prefers fewer evaluators and shorter batches.",
        f'{manifest["cpu_model"]}. {manifest["scope"]} Configuration and core assignments are included below.',
    ]
    if manifest.get("analysis_note"):
        notes.append(manifest["analysis_note"])
    return publish(directory, "Deployment frontier", table, ["frontier.png"], notes)
