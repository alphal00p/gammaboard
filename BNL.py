#!/usr/bin/env python3
"""Compute BNL complex-result ratios with first-order error propagation."""

from __future__ import annotations

import argparse
import json
import math
import re
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ComplexResult:
    re: float
    im: float
    re_err: float
    im_err: float

    @property
    def value(self) -> complex:
        return complex(self.re, self.im)


@dataclass(frozen=True)
class InputResult:
    value: ComplexResult
    source: str
    mt: float | None = None
    m_uv: float | None = None
    threshold_subtraction: bool | None = None
    threshold_class: str | None = None
    samples_per_piece: int | None = None
    base_workspace: str | None = None
    sector_workspace: str | None = None
    wall_seconds: float | None = None
    composition: str | None = None

    def setup_summary(self) -> str:
        fields = []
        if self.mt is not None:
            fields.append(f"MT=ymt={self.mt:g}")
        if self.m_uv is not None:
            fields.append(f"m_uv={self.m_uv:g}")
        if self.threshold_subtraction is not None:
            fields.append(
                "threshold_subtraction="
                + ("on" if self.threshold_subtraction else "off")
            )
        if self.threshold_class:
            fields.append(f"threshold_class={self.threshold_class}")
        if self.samples_per_piece is not None:
            fields.append(f"samples/piece={self.samples_per_piece:,}")
        if self.wall_seconds is not None:
            fields.append(f"wall={self.wall_seconds:.1f}s")
        if self.base_workspace:
            fields.append(f"base={self.base_workspace}")
        if self.sector_workspace:
            fields.append(f"sector={self.sector_workspace}")
        if self.composition:
            fields.append(self.composition)
        fields.append(f"source={self.source}")
        return "; ".join(fields)

    def as_json(self) -> dict[str, object]:
        return {
            **self.value.__dict__,
            "source": self.source,
            "mt": self.mt,
            "m_uv": self.m_uv,
            "threshold_subtraction": self.threshold_subtraction,
            "threshold_class": self.threshold_class,
            "samples_per_piece": self.samples_per_piece,
            "base_workspace": self.base_workspace,
            "sector_workspace": self.sector_workspace,
            "wall_seconds": self.wall_seconds,
            "composition": self.composition,
        }


def read_integration_result(path: Path) -> InputResult:
    with path.open() as f:
        data = json.load(f)
    slot = data["slots"][0]
    integral = slot["integral"]
    return InputResult(
        value=ComplexResult(
            re=float(integral["result"]["re"]),
            im=float(integral["result"]["im"]),
            re_err=float(integral["error"]["re"]),
            im_err=float(integral["error"]["im"]),
        ),
        source=str(path),
    )


def read_additive_summary(path: Path) -> dict[str, InputResult]:
    with path.open() as f:
        data = json.load(f)
    return {
        name: InputResult(
            value=ComplexResult(
                re=float(entry["physical_re"]),
                im=float(entry["physical_im"]),
                re_err=float(entry["error_re"]),
                im_err=float(entry["error_im"]),
            ),
            source=str(path),
            mt=float(entry["mt"]) if "mt" in entry else None,
            m_uv=float(entry["m_uv"]) if "m_uv" in entry else None,
            threshold_subtraction=(
                not bool(entry["disable_threshold_subtraction"])
                if "disable_threshold_subtraction" in entry
                else None
            ),
            threshold_class=entry.get("threshold_class"),
            samples_per_piece=int(entry["n"]) if "n" in entry else None,
            base_workspace=entry.get("base_ws"),
            sector_workspace=entry.get("sector_ws"),
            wall_seconds=(
                float(entry.get("wall_base", 0.0))
                + float(entry.get("wall_sector", 0.0))
                if "wall_base" in entry or "wall_sector" in entry
                else None
            ),
            composition=entry.get("composition"),
        )
        for name, entry in data.items()
    }


def ratio_with_uncertainty(num: ComplexResult, den: ComplexResult) -> ComplexResult:
    x = num.value
    y = den.value
    if y == 0:
        raise ZeroDivisionError("Cannot form a ratio with a zero denominator")

    value = x / y
    inv_y = 1.0 / y
    d_x_re = inv_y
    d_x_im = 1j * inv_y
    d_y_re = -x / (y * y)
    d_y_im = -1j * x / (y * y)

    derivatives = [
        (d_x_re, num.re_err),
        (d_x_im, num.im_err),
        (d_y_re, den.re_err),
        (d_y_im, den.im_err),
    ]
    re_var = sum((deriv.real * sigma) ** 2 for deriv, sigma in derivatives)
    im_var = sum((deriv.imag * sigma) ** 2 for deriv, sigma in derivatives)
    return ComplexResult(value.real, value.imag, re_var**0.5, im_var**0.5)


def parse_result_arg(raw: str) -> tuple[str, ComplexResult]:
    try:
        name, re, im, re_err, im_err = raw.split(":")
    except ValueError as exc:
        raise argparse.ArgumentTypeError(
            "manual results must be NAME:RE:IM:RE_ERR:IM_ERR"
        ) from exc
    return name, ComplexResult(float(re), float(im), float(re_err), float(im_err))


def manual_input_result(result: ComplexResult) -> InputResult:
    return InputResult(value=result, source="manual")


def _ndec(value: float, offset: int) -> int:
    ans = int(offset - math.log10(value))
    thresholds = [0.5, 9.5, 99.5]
    if ans > 0 and value * 10.0**ans >= thresholds[offset]:
        ans -= 1
    return max(ans, 0)


def _normalize_exponent(exponent: str) -> str:
    return str(int(exponent))


def _format_uncertainty(mean: float, error: float) -> str:
    value = mean
    delta = abs(error)

    if math.isnan(value) or math.isnan(delta):
        return f"{value:e} +/- {delta:e}"
    if math.isinf(delta):
        return f"{value:e} +/- inf"
    if value == 0.0 and not (1e-4 <= delta < 1e5):
        if delta == 0.0:
            return "0(0)"
        mantissa, exponent = f"{delta:.1e}".split("e")
        return f"0.0({mantissa})e{_normalize_exponent(exponent)}"
    if value == 0.0:
        if delta >= 9.95:
            return f"0({delta:.0f})"
        if delta >= 0.995:
            return f"0.0({delta:.1f})"
        decimals = _ndec(delta, 2)
        return f"{value:.{decimals}f}({delta * 10.0**decimals:.0f})"
    if delta == 0.0:
        mantissa, exponent = f"{value:e}".split("e")
        exponent = _normalize_exponent(exponent)
        return f"{mantissa}(0)e{exponent}" if exponent != "0" else f"{mantissa}(0)"
    if delta > 1e4 * abs(value):
        return f"{value:.1e} +/- {delta:.2e}"
    if abs(value) >= 1e6 or abs(value) < 1e-5:
        exponent = math.floor(math.log10(abs(value)))
        scale = 10.0**exponent
        mantissa = _format_uncertainty(value / scale, delta / scale)
        return f"{mantissa}e{exponent}"
    if delta >= 9.95:
        if abs(value) >= 9.5:
            return f"{value:.0f}({delta:.0f})"
        decimals = _ndec(abs(value), 1)
        return f"{value:.{decimals}f}({delta:.{decimals}f})"
    if delta >= 0.995:
        if abs(value) >= 0.95:
            return f"{value:.1f}({delta:.1f})"
        decimals = _ndec(abs(value), 1)
        return f"{value:.{decimals}f}({delta:.{decimals}f})"

    decimals = max(_ndec(abs(value), 1), _ndec(delta, 2))
    return f"{value:.{decimals}f}({delta * 10.0**decimals:.0f})"


def format_uncertainty(mean: float, error: float) -> str:
    formatted = _format_uncertainty(mean, error)
    return formatted if math.copysign(1.0, mean) < 0.0 else f"+{formatted}"


def format_result(result: ComplexResult) -> str:
    return f"{format_uncertainty(result.re, result.re_err)}  {format_uncertainty(result.im, result.im_err)} i"


def default_additive_summary_paths(root: Path) -> list[Path]:
    candidate_summary = root / "outputs" / "BNL_candidate_reference_scan_15min.json"
    if candidate_summary.exists():
        return [candidate_summary]
    return [
        root / "outputs" / "BNL_optimized_15min_scan.json",
        root / "outputs" / "BNL_optimized_extra_no_threshold_4M.json",
    ]


def default_ratios() -> list[str]:
    return [
        "R3/R6",
        "R3/R4",
        "R6/R4",
        "R6/R7",
        "R4/R7",
        "R4/R5",
        "R7/R5",
    ]


def default_reference_ratios() -> list[str]:
    return [
        "R1/R0",
        "R2/R0",
        "R3/R0",
        "R4/R0",
    ]


def parse_ratio_spec(raw: str) -> tuple[str, str]:
    if "/" not in raw:
        raise argparse.ArgumentTypeError("ratio specs must be NUM/DEN")
    numerator, denominator = raw.split("/", 1)
    if not numerator or not denominator:
        raise argparse.ArgumentTypeError("ratio specs must be NUM/DEN")
    return numerator, denominator


def ratio_corners(
    numerator: ComplexResult, denominator: ComplexResult, sigma_multiplier: float
) -> list[complex]:
    values = []
    for num_re_sign in (-1.0, 1.0):
        for num_im_sign in (-1.0, 1.0):
            for den_re_sign in (-1.0, 1.0):
                for den_im_sign in (-1.0, 1.0):
                    num = complex(
                        numerator.re
                        + num_re_sign * sigma_multiplier * numerator.re_err,
                        numerator.im
                        + num_im_sign * sigma_multiplier * numerator.im_err,
                    )
                    den = complex(
                        denominator.re
                        + den_re_sign * sigma_multiplier * denominator.re_err,
                        denominator.im
                        + den_im_sign * sigma_multiplier * denominator.im_err,
                    )
                    if den == 0.0:
                        continue
                    values.append(num / den)
    return values


def split_formatted_uncertainty(text: str) -> tuple[str, str]:
    if "(" in text:
        index = text.index("(")
        return text[:index], text[index:]
    marker = " +/- "
    if marker in text:
        index = text.index(marker)
        return text[:index], text[index:]
    return text, ""


def fixed_prefix(value: float, central_prefix: str, suffix: str) -> str:
    decimals = len(central_prefix.split(".", 1)[1]) if "." in central_prefix else 0
    match = re.search(r"e([+-]?\d+)", suffix)
    if match:
        value /= 10.0 ** int(match.group(1))
    return f"{value:+.{decimals}f}"


def stable_digit_positions(
    central_prefix: str, suffix: str, varied_values: list[float]
) -> set[int]:
    varied_prefixes = [
        fixed_prefix(value, central_prefix, suffix) for value in varied_values
    ]
    stable_positions = set()
    seen_significant_digit = False

    for index, char in enumerate(central_prefix):
        if not char.isdigit():
            continue
        if not seen_significant_digit:
            if char == "0":
                continue
            seen_significant_digit = True
        if all(index < len(prefix) and prefix[index] == char for prefix in varied_prefixes):
            stable_positions.add(index)
        else:
            break

    return stable_positions


def color_stable_digits(
    formatted: str, varied_values: list[float], use_color: bool
) -> tuple[str, int]:
    central_prefix, suffix = split_formatted_uncertainty(formatted)
    stable_positions = stable_digit_positions(central_prefix, suffix, varied_values)
    if not use_color or not stable_positions:
        return formatted, len(stable_positions)

    chunks = []
    in_green = False
    for index, char in enumerate(central_prefix):
        should_color = index in stable_positions
        if should_color and not in_green:
            chunks.append("\033[32m")
            in_green = True
        if not should_color and in_green:
            chunks.append("\033[0m")
            in_green = False
        chunks.append(char)
    if in_green:
        chunks.append("\033[0m")
    chunks.append(suffix)
    return "".join(chunks), len(stable_positions)


@dataclass(frozen=True)
class RatioDisplay:
    spec: str
    value: ComplexResult
    re_corner_values: list[float]
    im_corner_values: list[float]
    corner_sigma: float

    def as_json(self) -> dict[str, object]:
        return {
            **self.value.__dict__,
            "corner_sigma": self.corner_sigma,
            "re_corner_min": min(self.re_corner_values),
            "re_corner_max": max(self.re_corner_values),
            "im_corner_min": min(self.im_corner_values),
            "im_corner_max": max(self.im_corner_values),
            "re_stable_digit_count": stable_digit_count(
                self.value.re, self.value.re_err, self.re_corner_values
            ),
            "im_stable_digit_count": stable_digit_count(
                self.value.im, self.value.im_err, self.im_corner_values
            ),
        }


def ratio_display(
    spec: str,
    numerator: ComplexResult,
    denominator: ComplexResult,
    corner_sigma: float,
) -> RatioDisplay:
    value = ratio_with_uncertainty(numerator, denominator)
    corners = ratio_corners(numerator, denominator, corner_sigma)
    return RatioDisplay(
        spec=spec,
        value=value,
        re_corner_values=[corner.real for corner in corners],
        im_corner_values=[corner.imag for corner in corners],
        corner_sigma=corner_sigma,
    )


def stable_digit_count(value: float, error: float, varied_values: list[float]) -> int:
    formatted = format_uncertainty(value, error)
    central_prefix, suffix = split_formatted_uncertainty(formatted)
    return len(stable_digit_positions(central_prefix, suffix, varied_values))


def format_ratio_display(ratio: RatioDisplay, use_color: bool) -> str:
    re_text = format_uncertainty(ratio.value.re, ratio.value.re_err)
    im_text = format_uncertainty(ratio.value.im, ratio.value.im_err)
    re_text, _ = color_stable_digits(re_text, ratio.re_corner_values, use_color)
    im_text, _ = color_stable_digits(im_text, ratio.im_corner_values, use_color)
    return f"{re_text}  {im_text} i"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--result",
        action="append",
        type=parse_result_arg,
        default=[],
        metavar="NAME:RE:IM:RE_ERR:IM_ERR",
        help="Provide one manual complex result. Repeat for each ratio input.",
    )
    parser.add_argument(
        "--integration-result",
        action="append",
        default=[],
        metavar="NAME=PATH",
        help="Read one GammaLoop integration_result.json. Repeat for each ratio input.",
    )
    parser.add_argument(
        "--additive-summary",
        action="append",
        type=Path,
        default=[],
        help=(
            "Read a JSON summary where each R entry contains physical_re, "
            "physical_im, error_re and error_im. Repeat to merge summaries."
        ),
    )
    parser.add_argument(
        "--ratio",
        action="append",
        type=parse_ratio_spec,
        default=[],
        metavar="NUM/DEN",
        help="Ratio to print. Defaults to the selected stable BNL ratios.",
    )
    parser.add_argument(
        "--no-color",
        action="store_true",
        help="Do not color leading digits that are stable under the corner scan.",
    )
    parser.add_argument(
        "--corner-sigma",
        type=float,
        default=3.0,
        help="Sigma multiplier for the 16 corner configurations. Defaults to 3.",
    )
    parser.add_argument(
        "--json-output",
        type=Path,
        help="Optional path where the ratio summary is written as JSON.",
    )
    args = parser.parse_args()

    root = Path(__file__).resolve().parent
    inputs: dict[str, InputResult] = {}

    summary_paths = args.additive_summary
    if not args.result and not args.integration_result and not summary_paths:
        summary_paths = default_additive_summary_paths(root)

    for summary_path in summary_paths:
        inputs.update(read_additive_summary(summary_path))

    for name, result in args.result:
        inputs[name] = manual_input_result(result)

    for raw in args.integration_result:
        if "=" not in raw:
            raise SystemExit("--integration-result entries must be NAME=PATH")
        name, raw_path = raw.split("=", 1)
        inputs[name] = read_integration_result(Path(raw_path))

    default_ratio_specs = default_reference_ratios() if "R0" in inputs else default_ratios()
    ratio_specs = args.ratio or [parse_ratio_spec(spec) for spec in default_ratio_specs]
    needed_names = sorted({name for spec in ratio_specs for name in spec})
    missing = [name for name in needed_names if name not in inputs]
    if missing:
        raise SystemExit(f"Missing required results: {', '.join(missing)}")

    ratios = {
        f"{numerator}/{denominator}": ratio_display(
            f"{numerator}/{denominator}",
            inputs[numerator].value,
            inputs[denominator].value,
            args.corner_sigma,
        )
        for numerator, denominator in ratio_specs
    }

    print("Input setups:")
    for name in needed_names:
        print(f"  {name}: {format_result(inputs[name].value)}")
        print(f"      {inputs[name].setup_summary()}")

    print("\nRatios:")
    print(
        "  green: leading digits unchanged over the 16 "
        f"{args.corner_sigma:g}-sigma real/imag corner ratios"
    )
    for name, ratio in ratios.items():
        print(f"  {name}: {format_ratio_display(ratio, not args.no_color)}")

    if args.json_output:
        payload = {
            "inputs": {name: result.as_json() for name, result in inputs.items()},
            "corner_sigma": args.corner_sigma,
            "ratios": {name: ratio.as_json() for name, ratio in ratios.items()},
        }
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(json.dumps(payload, indent=2))


if __name__ == "__main__":
    main()
