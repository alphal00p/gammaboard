"""Separate figures and data, with lightweight local HTML navigation."""

import csv
import html
import json


def publish(directory, title, rows, figures, notes):
    manifest = json.loads((directory / "manifest.json").read_text())
    if rows:
        with (directory / "summary.csv").open("w", newline="") as stream:
            writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)

    def esc(value):
        return html.escape(str(value))

    def fmt(value):
        if value is None:
            return "—"
        return f"{value:,.3g}" if isinstance(value, float) else esc(value)

    table = ""
    if rows:
        table = (
            '<div class="table"><table><thead><tr>'
            + "".join(f'<th>{esc(k.replace("_"," "))}</th>' for k in rows[0])
            + "</tr></thead><tbody>"
        )
        table += "".join(
            "<tr>" + "".join(f"<td>{fmt(v)}</td>" for v in row.values()) + "</tr>" for row in rows
        )
        table += "</tbody></table></div>"
    status = manifest.get("status", "unknown")
    content = f'<h1>{esc(title)}</h1><p class="status">{esc(status)} · {manifest.get("elapsed_seconds",0):.1f} seconds</p>'
    content += "".join(f"<p>{esc(note)}</p>" for note in notes)
    for name in figures:
        path = directory / name
        if path.exists():
            content += f'<p><a href="{esc(name)}">PNG</a> · <a href="{esc(path.with_suffix(".svg").name)}">SVG</a> · <a href="summary.csv">CSV</a></p>'
            content += f'<img alt="{esc(path.stem)}" src="{esc(name)}">'
    content += table
    content += (
        "<details><summary>Configuration and provenance</summary><pre>"
        + esc(json.dumps(manifest, indent=2))
        + "</pre></details>"
    )
    write_html(directory / "report.html", title, content)
    return directory / "report.html"


def write_html(path, title, content):
    path.write_text(
        f"""<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width">
<title>{html.escape(title)}</title><style>
body{{font:16px system-ui,sans-serif;color:#243248;background:#f3f5f8;margin:0}}
main{{max-width:1150px;margin:32px auto;padding:32px;background:white;border-radius:12px}}
h1{{font-size:28px}}p{{line-height:1.6}}.status{{color:#52667b}}img{{display:block;width:100%;height:auto;margin:24px 0}}
.table{{overflow:auto}}table{{border-collapse:collapse;width:100%;font-size:14px}}th,td{{padding:10px;text-align:right;border-bottom:1px solid #dde3ea;white-space:nowrap}}th{{background:#edf2f7}}th:first-child,td:first-child{{text-align:left}}details{{margin-top:24px}}pre{{white-space:pre-wrap;overflow-wrap:anywhere}}section{{margin-bottom:48px}}a{{color:#185bbb}}
</style><main>{content}</main></html>"""
    )


def aggregate(directory, children):
    manifest = json.loads((directory / "manifest.json").read_text())
    status = html.escape(manifest.get("status", "unknown"))
    sections = [
        f'<h1>GammaBoard benchmark suite</h1><p class="status">{status} · '
        f'{manifest.get("elapsed_seconds",0):.1f} seconds</p>'
    ]
    for child in children:
        report = child / "report.html"
        if report.exists():
            metadata = (
                json.loads((child / "manifest.json").read_text())
                if (child / "manifest.json").exists()
                else {}
            )
            label = html.escape(child.name)
            state = html.escape(metadata.get("status", "unknown"))
            sections.append(
                f'<p><a href="{label}/report.html">{label}</a> · {state} · <a href="{label}/summary.csv">CSV</a></p>'
            )
    write_html(directory / "report.html", "GammaBoard benchmark suite", "".join(sections))
    return directory / "report.html"


def figure_save(fig, directory, name):
    for ext in ("png", "svg"):
        fig.savefig(directory / f"{name}.{ext}", dpi=160, bbox_inches="tight")


def batch_axis(ax, sizes):
    """Keep logarithmic spacing, but label the actual batch sizes."""
    ax.set_xscale("log", base=2)
    ax.set_xticks(
        sizes,
        labels=[f"{n // 1024}k" if n >= 1024 and n % 1024 == 0 else str(n) for n in sizes],
    )
    ax.set_xlabel("Samples per batch (k = 1,024)")
