from setuptools import find_packages, setup


setup(
    name="gammaboard-process",
    version="0.1.0",
    description="Python helpers for GammaBoard process evaluators and samplers",
    package_dir={"": "src"},
    packages=find_packages("src"),
    python_requires=">=3.11",
    install_requires=["numpy"],
)
