import os


def main() -> None:
    # Set the default before importing the numerical libraries.
    os.environ.setdefault("OMP_NUM_THREADS", "1")
    from madnis_sampler import MadnisSampler
    from gammaboard_process import run_sampler

    run_sampler(MadnisSampler)


if __name__ == "__main__":
    main()
