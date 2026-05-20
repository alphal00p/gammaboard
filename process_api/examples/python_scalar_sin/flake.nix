{
  description = "Minimal flake runtime for a numpy-backed scalar integrand";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      gammaboardProcess = builtins.path {
        path = ../../python;
        name = "gammaboard-process-python";
      };
      pythonEnv = pkgs.python311.withPackages (ps: [ ps.numpy ]);
      runtimePython = pkgs.writeShellScriptBin "python" ''
        export PYTHONPATH="${./src}:${gammaboardProcess}/src:$PYTHONPATH"
        exec ${pythonEnv}/bin/python "$@"
      '';
      evaluatorWorker = pkgs.writeShellScriptBin "gammaboard-example-evaluator-worker" ''
        export PYTHONPATH="${./src}:${gammaboardProcess}/src:$PYTHONPATH"
        exec ${pythonEnv}/bin/python -u ${./evaluator_worker.py} "$@"
      '';
    in {
      packages.${system}.runtime = pkgs.symlinkJoin {
        name = "python-scalar-sin-runtime";
        paths = [ runtimePython evaluatorWorker pythonEnv ];
      };
    };
}
