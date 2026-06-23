from demo_sampler import SymbolicaHavanaSampler
from gammaboard_process import run_sampler


def main() -> None:
    run_sampler(SymbolicaHavanaSampler)


if __name__ == "__main__":
    main()
