from demo_integrand import SinIntegrand
from gammaboard_process import run_evaluator


def main() -> None:
    run_evaluator(SinIntegrand)


if __name__ == "__main__":
    main()
