from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent / "src"))

from demo_integrand import SinIntegrand
from gammaboard_process import run_evaluator


if __name__ == "__main__":
    run_evaluator(SinIntegrand)
