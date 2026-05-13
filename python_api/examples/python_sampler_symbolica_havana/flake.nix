{
  description = "Minimal flake runtime for a Symbolica-backed Havana sampler example";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      python = pkgs.python311;
      symbolica = python.pkgs.buildPythonPackage rec {
        pname = "symbolica";
        version = "1.5.1";
        format = "wheel";
        src = pkgs.fetchurl {
          url = "https://files.pythonhosted.org/packages/17/39/8b5b1cee183c6b96ba68175da260e646daa12515bb3791afd92890ea0fb5/symbolica-1.5.1-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl";
          hash = "sha256-9ge/6fpKSzhlGtCUsalE0XH0BAHDgG6RzMXUFGzqs0c=";
        };
        doCheck = false;
      };
      pythonEnv = python.withPackages (ps: [
        ps.numpy
        symbolica
      ]);
      runtimePython = pkgs.writeShellScriptBin "python" ''
        export PYTHONPATH="${./src}:$PYTHONPATH"
        exec ${pythonEnv}/bin/python "$@"
      '';
      samplerWorker = pkgs.writeShellScriptBin "gammaboard-example-sampler-worker" ''
        export PYTHONPATH="${./src}:$PYTHONPATH"
        exec ${pythonEnv}/bin/python -u ${./sampler_worker.py} "$@"
      '';
    in {
      packages.${system}.runtime = pkgs.symlinkJoin {
        name = "python-sampler-symbolica-havana-runtime";
        paths = [ runtimePython samplerWorker pythonEnv ];
      };
    };
}
