#!/usr/bin/env python3
"""Convert 0.2 run definitions to 0.3; print a reviewable copy by default.

Requires tomlkit. Never edits the input, follows external files, or migrates state.
"""
from __future__ import annotations

import argparse
from collections.abc import MutableMapping
from pathlib import Path
import sys

import tomlkit


def convert(document):
    changes = []

    def note(path, message):
        changes.append(f"{path}: {message}")

    def visit(value, path="run", evaluator_kind=None):
        if isinstance(value, list):
            for index, item in enumerate(value):
                visit(item, f"{path}[{index}]", evaluator_kind)
            return
        if not isinstance(value, MutableMapping):
            return
        evaluator = value.get("evaluator", {})
        if isinstance(evaluator, MutableMapping):
            evaluator_kind = evaluator.get("kind", evaluator.get("config", {}).get("kind", evaluator_kind))
        if "sampler_aggregator" in value:
            if "sampler" in value:
                raise ValueError(f"{path}: both sampler and sampler_aggregator are present")
            value["sampler"] = value.pop("sampler_aggregator")
            note(path, "renamed sampler_aggregator to sampler")
        if value.get("kind") == "symbolica":
            source = value.get("source")
            if source is not None and "expr" in value:
                raise ValueError(f"{path}: both expr and source are present; choose the intended source")
            if source is None:
                if "expr" not in value:
                    raise ValueError(f"{path}: Symbolica source is missing")
                source = tomlkit.inline_table()
                source.update(kind="expression", expr=value.pop("expr"))
                value["source"] = source
                note(path, "moved expression into a tagged source")
            # Assignment converts dicts to TOML items; use the stored table.
            source = value["source"]
            for field in ("constants", "compiled_args", "bindings"):
                if field in value:
                    if field in source:
                        raise ValueError(f"{path}: duplicate source.{field}")
                    if (field == "constants") != (source.get("kind") == "expression"):
                        raise ValueError(f"{path}: {field} is incompatible with this Symbolica source")
                    source[field] = value.pop(field)
                    note(path, f"moved {field} into source")
        if value.get("kind") == "process_evaluator" and "output" not in value:
            accumulator = value.pop("accumulator", "vector")
            components = value.pop("components", ["value"])
            output = tomlkit.inline_table()
            output["kind"] = "gammaloop" if accumulator == "gammaloop" else "values"
            if output["kind"] == "values":
                output["components"] = components
            value["output"] = output
            note(path, "declared process output; rebuild the worker against SDK 0.3")
        if value.get("kind") == "process_sampler" and "requires_training_values" in value:
            value.pop("requires_training_values")
            note(path, "removed requires_training_values; the v6 generate callback must declare training windows")
        if set(value) == {"continuous"}:
            dims = value.pop("continuous")["dims"]
            rectangular = tomlkit.inline_table()
            rectangular.update(discrete_cardinalities=[], continuous_dims=dims)
            value["rectangular"] = rectangular
            note(path, "converted continuous domain to rectangular")
        if value.get("kind") == "sample" and "nr_samples" in value:
            if "stop_condition" in value:
                raise ValueError(f"{path}: both nr_samples and stop_condition are present")
            value["stop_condition"] = {"max_samples": value.pop("nr_samples")}
            note(path, "converted sample budget to stop_condition")
        for field in ("measurement", "objective"):
            measurement = value.get(field)
            if not isinstance(measurement, MutableMapping):
                continue
            if "selection" in measurement and any(k in measurement for k in ("metric", "quantity")):
                raise ValueError(f"{path}.{field}: mixed old and new measurement selection")
            metric = measurement.pop("metric", None)
            quantity = measurement.pop("quantity", None)
            if metric is not None and quantity not in (None, "central_value"):
                raise ValueError(f"{path}.{field}: ambiguous metric and quantity")
            if metric is not None or isinstance(quantity, MutableMapping):
                source = metric if metric is not None else quantity
                selected = {"kind": "metric", "name": source} if isinstance(source, str) else {"kind": "metric", **source}
                if "metric" in selected:
                    selected["name"] = selected.pop("metric")
                measurement["selection"] = selected
                note(path, f"converted {field} metric selection")
            elif quantity is not None:
                if quantity != "central_value":
                    raise ValueError(f"{path}.{field}: unsupported quantity {quantity!r}")
                measurement["selection"] = {"kind": "all_component_means"}
        stop = value.get("stop_condition")
        if isinstance(stop, MutableMapping) and "projection" in stop:
            if "metric" in stop or evaluator_kind != "gammaloop":
                raise ValueError(f"{path}: projection needs manual review against the effective evaluator; use an explicit metric selector")
            component = {"real": "real", "imag": "imag", "abs": "norm"}[stop.pop("projection")]
            stop["metric"] = {"name": "mean", "component": component}
            note(path, "converted GammaLoop stop projection to a metric")
        sampler = value.get("sampler")
        if value.get("kind") == "sample" and isinstance(sampler, MutableMapping) and "config" in sampler and "batch_transforms" in value:
            if "transforms" in sampler["config"]:
                raise ValueError(f"{path}: duplicate transform settings")
            sampler["config"]["transforms"] = value.pop("batch_transforms")
            note(path, "moved transforms into the explicit sampling pipeline")
        for key, child in list(value.items()):
            if key == "run" and isinstance(child, str):
                nested, nested_changes = convert(tomlkit.parse(child))
                if nested_changes:
                    value[key] = tomlkit.dumps(nested)
                    changes.extend(f"{path}.run: {change}" for change in nested_changes)
                continue
            if key not in ("args", "constants", "bindings", "replacements", "metadata"):
                visit(child, f"{path}.{key}", evaluator_kind)

    visit(document)
    # Rebuild containers after moving fields: preserved inline-table trivia can
    # otherwise retain obsolete commas when an interior field is removed.
    clean = tomlkit.document()
    clean.update(document.unwrap())
    return clean, changes


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, help="write a new file instead of printing the dry-run result")
    args = parser.parse_args(argv)
    try:
        document, changes = convert(tomlkit.parse(args.input.read_text()))
        result = tomlkit.dumps(document)
        if args.output:
            # Never overwrite the original or an existing output.
            with args.output.open("x") as handle:
                handle.write(result)
        else:
            print(result, end="")
        for change in changes:
            print(change, file=sys.stderr)
        print("Review and run gammaboard run validate on the result. Convert referenced child files separately; rebuild process callbacks. Checkpoints are not converted.", file=sys.stderr)
    except (ValueError, KeyError, OSError, tomlkit.exceptions.ParseError) as error:
        parser.exit(2, f"conversion failed: {error}\n")


if __name__ == "__main__":
    main()
