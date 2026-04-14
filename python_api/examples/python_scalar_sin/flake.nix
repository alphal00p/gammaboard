{
  description = "Minimal flake runtime for a numpy-backed scalar integrand";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      pythonEnv = pkgs.python311.withPackages (ps: [ ps.numpy ]);
    in {
      packages.${system}.runtime = pkgs.symlinkJoin {
        name = "python-scalar-sin-runtime";
        paths = [ pythonEnv ./src ];
      };
    };
}
